//! 净化后的 AI 容量快照（只读导出）
//!
//! 从 Cockpit 已有的账号/配额数据生成机器可读的规范化快照，供外部调度器
//! （Multica Lead / ChatGPT Supervisor）做配额感知路由。
//!
//! 安全边界：
//! - 只输出白名单字段；绝不输出 token / API key / cookie / 邮箱等凭据材料。
//! - `credential_ref` 是不透明的逻辑引用，Cockpit 仍是唯一凭据持有者。
//! - 配额错误只保留错误码与时间戳，不透传原始错误消息。

use crate::models::codex::{CodexAccount, CodexAuthMode};
use crate::models::Account;
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

/// 快照结构版本
pub const CAPACITY_SNAPSHOT_SCHEMA_VERSION: &str = "1.0";
/// 默认快照 TTL（秒）
pub const DEFAULT_TTL_SECONDS: i64 = 300;
/// 快照来源标识
pub const SNAPSHOT_SOURCE: &str = "cockpit-tools";

/// 路由健康状态
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum HealthStatus {
    Healthy,
    Degraded,
    Unavailable,
}

/// 整份快照是否可供调度消费。
/// `unavailable` 表示数据源全部失败，调用方必须回退静态路由，而不是猜测配额。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum SnapshotAvailability {
    Ok,
    Degraded,
    Unavailable,
}

/// 配额错误分类（用于健康归因，只携带错误码）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ErrorKind {
    Auth,
    RateLimit,
}

/// 规范化的配额窗口
#[derive(Debug, Clone, Serialize)]
pub struct QuotaWindowSnapshot {
    /// 窗口名（模型 ID 或 primary_5h / weekly）
    pub name: String,
    /// 剩余比例 0.0-1.0
    pub remaining_ratio: f64,
    /// 重置时间（RFC3339 UTC）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reset_at: Option<String>,
    /// 窗口时长（分钟，仅 Codex 等提供时存在）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub window_minutes: Option<i64>,
}

/// 路由健康信息
#[derive(Debug, Clone, Serialize)]
pub struct RouteHealth {
    pub status: HealthStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_auth_error_at: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_rate_limit_at: Option<i64>,
    /// 预留给消费方的临时反馈冷却；Cockpit 本身不产生该值
    pub cooldown_until: Option<i64>,
    /// 净化后的错误码（如 "401"、"429"、"invalid_grant"），不含错误正文
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail_code: Option<String>,
}

/// 单条路由的净化容量条目
#[derive(Debug, Clone, Serialize)]
pub struct RouteCapacity {
    /// 稳定路由 ID："{provider}:{account_alias}"
    pub route_id: String,
    /// 提供商（antigravity / codex）
    pub provider: String,
    /// 稳定且不透明的账号别名（由账号 ID 派生，不含邮箱/原始 ID）
    pub account_alias: String,
    /// 不透明凭据引用；凭据本体始终留在 Cockpit 内
    pub credential_ref: String,
    /// 是否为该提供商当前激活账号
    pub is_current: bool,
    /// 订阅/服务层级
    #[serde(skip_serializing_if = "Option::is_none")]
    pub plan: Option<String>,
    pub quota_windows: Vec<QuotaWindowSnapshot>,
    pub health: RouteHealth,
    /// 数据更新时间（Unix 秒）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<i64>,
    /// 非秘密元数据（用户标签、层级 ID、积分摘要等）
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub metadata: BTreeMap<String, serde_json::Value>,
}

/// 单个数据源读取状态
#[derive(Debug, Clone, Serialize)]
pub struct SourceStatus {
    pub provider: String,
    pub ok: bool,
}

fn availability_from_sources(sources: &[SourceStatus]) -> SnapshotAvailability {
    if sources.is_empty() {
        return SnapshotAvailability::Unavailable;
    }
    let any_ok = sources.iter().any(|s| s.ok);
    let all_ok = sources.iter().all(|s| s.ok);
    if all_ok {
        SnapshotAvailability::Ok
    } else if any_ok {
        SnapshotAvailability::Degraded
    } else {
        SnapshotAvailability::Unavailable
    }
}

/// 净化容量快照
#[derive(Debug, Clone, Serialize)]
pub struct CapacitySnapshot {
    pub schema_version: String,
    /// 快照生成时间（Unix 秒）
    pub generated_at: i64,
    pub ttl_seconds: i64,
    pub source: &'static str,
    pub availability: SnapshotAvailability,
    pub sources: Vec<SourceStatus>,
    pub routes: Vec<RouteCapacity>,
}

/// 由账号 ID 派生稳定的 8 位十六进制别名后缀
fn opaque_suffix(provider: &str, id: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(provider.as_bytes());
    hasher.update(b":");
    hasher.update(id.as_bytes());
    let digest = hasher.finalize();
    digest[..4].iter().map(|b| format!("{:02x}", b)).collect()
}

fn account_alias_for(provider: &str, id: &str) -> String {
    format!("{}-{}", provider, opaque_suffix(provider, id))
}

fn credential_ref_for(provider: &str, id: &str) -> String {
    format!("cockpit:{}:{}", provider, opaque_suffix(provider, id))
}

fn route_id_for(provider: &str, alias: &str) -> String {
    format!("{}:{}", provider, alias)
}

fn ratio_from_percentage(percentage: i32) -> f64 {
    (percentage.clamp(0, 100) as f64) / 100.0
}

fn unix_to_rfc3339(secs: i64) -> Option<String> {
    chrono::DateTime::from_timestamp(secs, 0)
        .map(|dt| dt.to_rfc3339_opts(chrono::SecondsFormat::Secs, true))
}

/// 归一化 Antigravity 的 reset_time 字符串（RFC3339 或 Unix 秒字符串）。
/// 无法解析时返回 None，不透传原文。
fn normalize_reset_time_str(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(trimmed) {
        return Some(dt.to_rfc3339_opts(chrono::SecondsFormat::Secs, true));
    }
    if let Ok(secs) = trimmed.parse::<i64>() {
        return unix_to_rfc3339(secs);
    }
    None
}

/// 按错误码/错误文本归类错误类型（仅用于健康标记）
fn classify_error(code: Option<&str>, message: &str) -> Option<ErrorKind> {
    let code_lower = code.unwrap_or("").to_lowercase();
    let msg_lower = message.to_lowercase();

    const AUTH_CODES: [&str; 6] = [
        "401",
        "403",
        "unauthorized",
        "forbidden",
        "invalid_grant",
        "unauthenticated",
    ];
    const RATE_CODES: [&str; 4] = [
        "429",
        "resource_exhausted",
        "rate_limit",
        "rate_limit_exceeded",
    ];

    if AUTH_CODES.contains(&code_lower.as_str()) {
        return Some(ErrorKind::Auth);
    }
    if RATE_CODES.contains(&code_lower.as_str()) {
        return Some(ErrorKind::RateLimit);
    }
    const AUTH_HINTS: [&str; 3] = ["unauthorized", "invalid_grant", "permission denied"];
    if AUTH_HINTS.iter().any(|h| msg_lower.contains(h)) {
        return Some(ErrorKind::Auth);
    }
    const RATE_HINTS: [&str; 2] = ["rate limit", "resource_exhausted"];
    if RATE_HINTS.iter().any(|h| msg_lower.contains(h)) {
        return Some(ErrorKind::RateLimit);
    }
    None
}

fn sanitize_error_code(code: Option<&str>) -> Option<String> {
    code.map(str::trim)
        .filter(|c| !c.is_empty())
        .filter(|c| {
            c.chars()
                .all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '-')
        })
        .map(str::to_string)
}

fn health_from(
    disabled: bool,
    is_forbidden: bool,
    error: Option<(Option<&str>, &str, i64)>,
) -> RouteHealth {
    let mut health = RouteHealth {
        status: HealthStatus::Healthy,
        last_auth_error_at: None,
        last_rate_limit_at: None,
        cooldown_until: None,
        detail_code: None,
    };

    if disabled || is_forbidden {
        health.status = HealthStatus::Unavailable;
        if is_forbidden {
            health.detail_code = Some("forbidden".to_string());
        } else {
            health.detail_code = Some("disabled".to_string());
        }
        return health;
    }

    if let Some((code, message, at)) = error {
        health.detail_code = sanitize_error_code(code);
        match classify_error(code, message) {
            Some(ErrorKind::Auth) => {
                health.status = HealthStatus::Degraded;
                health.last_auth_error_at = Some(at);
            }
            Some(ErrorKind::RateLimit) => {
                health.status = HealthStatus::Degraded;
                health.last_rate_limit_at = Some(at);
            }
            None => {}
        }
    }

    health
}

fn insert_tags(metadata: &mut BTreeMap<String, serde_json::Value>, tags: &[String]) {
    if !tags.is_empty() {
        metadata.insert(
            "tags".to_string(),
            serde_json::Value::Array(
                tags.iter()
                    .map(|t| serde_json::Value::String(t.clone()))
                    .collect(),
            ),
        );
    }
}

fn insert_optional_tags(
    metadata: &mut BTreeMap<String, serde_json::Value>,
    tags: Option<&Vec<String>>,
) {
    if let Some(tags) = tags {
        insert_tags(metadata, tags);
    }
}

/// 从 Antigravity 账号构建净化路由条目
pub fn route_from_antigravity(account: &Account, is_current: bool) -> RouteCapacity {
    let provider = "antigravity";
    let alias = account_alias_for(provider, &account.id);
    let mut quota_windows = Vec::new();
    let mut updated_at = account.usage_updated_at;

    if let Some(quota) = account.quota.as_ref() {
        updated_at = updated_at.max(Some(quota.last_updated));
        for model in &quota.models {
            quota_windows.push(QuotaWindowSnapshot {
                name: model.name.clone(),
                remaining_ratio: ratio_from_percentage(model.percentage),
                reset_at: normalize_reset_time_str(&model.reset_time),
                window_minutes: None,
            });
        }
    }

    let mut metadata = BTreeMap::new();
    insert_tags(&mut metadata, &account.tags);
    if let Some(quota) = account.quota.as_ref() {
        if let Some(tier_id) = quota.tier_id.as_deref().filter(|t| !t.is_empty()) {
            metadata.insert(
                "tier_id".to_string(),
                serde_json::Value::String(tier_id.to_string()),
            );
        }
        if !quota.credits.is_empty() {
            let credits: Vec<serde_json::Value> = quota
                .credits
                .iter()
                .map(|c| {
                    serde_json::json!({
                        "credit_type": c.credit_type,
                        "credit_amount": c.credit_amount,
                    })
                })
                .collect();
            metadata.insert("credits".to_string(), serde_json::Value::Array(credits));
        }
    }

    let is_forbidden = account
        .quota
        .as_ref()
        .map(|q| q.is_forbidden)
        .unwrap_or(false);
    let quota_error = account.quota_error.as_ref().map(|e| {
        let code = e.code.map(|c| c.to_string());
        (code, e.message.clone(), e.timestamp)
    });
    let health = match &quota_error {
        Some((code, message, ts)) => health_from(
            account.disabled,
            is_forbidden,
            Some((code.as_deref(), message.as_str(), *ts)),
        ),
        None => health_from(account.disabled, is_forbidden, None),
    };

    RouteCapacity {
        route_id: route_id_for(provider, &alias),
        provider: provider.to_string(),
        account_alias: alias.clone(),
        credential_ref: credential_ref_for(provider, &account.id),
        is_current,
        plan: account
            .quota
            .as_ref()
            .and_then(|q| q.subscription_tier.clone()),
        quota_windows,
        health,
        updated_at,
        metadata,
    }
}

/// 从 Codex 账号构建净化路由条目
pub fn route_from_codex(account: &CodexAccount, is_current: bool) -> RouteCapacity {
    let provider = "codex";
    let alias = account_alias_for(provider, &account.id);
    let mut quota_windows = Vec::new();

    if let Some(quota) = account.quota.as_ref() {
        if quota.hourly_window_present.unwrap_or(true) {
            quota_windows.push(QuotaWindowSnapshot {
                name: "primary_5h".to_string(),
                remaining_ratio: ratio_from_percentage(quota.hourly_percentage),
                reset_at: quota.hourly_reset_time.and_then(unix_to_rfc3339),
                window_minutes: quota.hourly_window_minutes,
            });
        }
        if quota.weekly_window_present.unwrap_or(true) {
            quota_windows.push(QuotaWindowSnapshot {
                name: "weekly".to_string(),
                remaining_ratio: ratio_from_percentage(quota.weekly_percentage),
                reset_at: quota.weekly_reset_time.and_then(unix_to_rfc3339),
                window_minutes: quota.weekly_window_minutes,
            });
        }
    }

    let mut metadata = BTreeMap::new();
    insert_optional_tags(&mut metadata, account.tags.as_ref());
    if account.auth_mode == CodexAuthMode::Apikey {
        metadata.insert(
            "auth_mode".to_string(),
            serde_json::Value::String("apikey".to_string()),
        );
    }

    let health = health_from(
        false,
        false,
        account
            .quota_error
            .as_ref()
            .map(|e| (e.code.as_deref(), e.message.as_str(), e.timestamp)),
    );

    RouteCapacity {
        route_id: route_id_for(provider, &alias),
        provider: provider.to_string(),
        account_alias: alias.clone(),
        credential_ref: credential_ref_for(provider, &account.id),
        is_current,
        plan: account.plan_type.clone(),
        quota_windows,
        health,
        updated_at: account.usage_updated_at,
        metadata,
    }
}

/// 从内存中的账号列表组装快照（测试与离线复用的纯函数入口）。
/// 注意：本函数只读取配额与健康字段，任何凭据字段都不会进入快照。
pub fn build_snapshot_from_parts(
    antigravity_accounts: &[Account],
    antigravity_current_id: Option<&str>,
    codex_accounts: &[CodexAccount],
    codex_current_id: Option<&str>,
) -> CapacitySnapshot {
    let mut routes = Vec::new();
    for account in antigravity_accounts {
        let is_current = antigravity_current_id == Some(account.id.as_str());
        routes.push(route_from_antigravity(account, is_current));
    }
    for account in codex_accounts {
        let is_current = codex_current_id == Some(account.id.as_str());
        routes.push(route_from_codex(account, is_current));
    }

    let sources = vec![
        SourceStatus {
            provider: "antigravity".to_string(),
            ok: true,
        },
        SourceStatus {
            provider: "codex".to_string(),
            ok: true,
        },
    ];
    CapacitySnapshot {
        schema_version: CAPACITY_SNAPSHOT_SCHEMA_VERSION.to_string(),
        generated_at: chrono::Utc::now().timestamp(),
        ttl_seconds: DEFAULT_TTL_SECONDS,
        source: SNAPSHOT_SOURCE,
        availability: availability_from_sources(&sources),
        sources,
        routes,
    }
}

/// 读取本地已有账号/配额数据并生成净化快照（只读，不触发网络刷新）。
/// 账号详情文件支持 AES-256-GCM 信封与历史明文两种格式；单个数据源或单条账号
/// 失败不会阻断其他数据。
pub fn build_capacity_snapshot() -> CapacitySnapshot {
    let mut routes = Vec::new();
    let mut sources = Vec::new();

    match load_antigravity_accounts() {
        Ok((accounts, current_id)) => {
            for account in &accounts {
                let is_current = current_id.as_deref() == Some(account.id.as_str());
                routes.push(route_from_antigravity(account, is_current));
            }
            sources.push(SourceStatus {
                provider: "antigravity".to_string(),
                ok: true,
            });
        }
        Err(_) => {
            sources.push(SourceStatus {
                provider: "antigravity".to_string(),
                ok: false,
            });
        }
    }

    match load_codex_accounts() {
        Ok((accounts, current_id)) => {
            for account in &accounts {
                let is_current = current_id.as_deref() == Some(account.id.as_str());
                routes.push(route_from_codex(account, is_current));
            }
            sources.push(SourceStatus {
                provider: "codex".to_string(),
                ok: true,
            });
        }
        Err(_) => {
            sources.push(SourceStatus {
                provider: "codex".to_string(),
                ok: false,
            });
        }
    }

    CapacitySnapshot {
        schema_version: CAPACITY_SNAPSHOT_SCHEMA_VERSION.to_string(),
        generated_at: chrono::Utc::now().timestamp(),
        ttl_seconds: DEFAULT_TTL_SECONDS,
        source: SNAPSHOT_SOURCE,
        availability: availability_from_sources(&sources),
        sources,
        routes,
    }
}

/// 只读加载 Antigravity 账号列表：索引 + 详情文件（自动兼容加密信封），
/// 并像 GUI 一样合并本地配额缓存。不触发索引修复等任何写操作。
fn load_antigravity_accounts() -> Result<(Vec<Account>, Option<String>), String> {
    let data_dir = crate::modules::config::get_data_dir()?;
    let index_path = data_dir.join("accounts.json");
    if !index_path.exists() {
        return Ok((Vec::new(), None));
    }
    let content =
        std::fs::read_to_string(&index_path).map_err(|e| format!("读取账号索引失败: {}", e))?;
    let index: crate::models::AccountIndex =
        serde_json::from_str(&content).map_err(|e| format!("解析账号索引失败: {}", e))?;

    let accounts_dir = data_dir.join("accounts");
    let mut accounts = Vec::new();
    for summary in &index.accounts {
        let path = accounts_dir.join(format!("{}.json", summary.id));
        let Ok(file_content) = std::fs::read_to_string(&path) else {
            continue;
        };
        let Ok(mut account) = crate::modules::secure_account_reader::read_account_detail::<Account>(
            &path,
            &file_content,
        ) else {
            continue;
        };
        let _ = crate::modules::quota_cache::apply_cached_quota(&mut account, "authorized");
        accounts.push(account);
    }
    let current_id = crate::modules::account::get_current_account_id()
        .ok()
        .flatten();
    Ok((accounts, current_id))
}

/// 只读加载 Codex 账号列表，逻辑同上。
fn load_codex_accounts() -> Result<(Vec<CodexAccount>, Option<String>), String> {
    let data_dir = crate::modules::config::get_data_dir()?;
    let index_path = data_dir.join("codex_accounts.json");
    if !index_path.exists() {
        return Ok((Vec::new(), None));
    }
    let content =
        std::fs::read_to_string(&index_path).map_err(|e| format!("读取 Codex 索引失败: {}", e))?;
    let index: crate::models::codex::CodexAccountIndex =
        serde_json::from_str(&content).map_err(|e| format!("解析 Codex 索引失败: {}", e))?;

    let accounts_dir = data_dir.join("codex_accounts");
    let mut accounts = Vec::new();
    for summary in &index.accounts {
        let path = accounts_dir.join(format!("{}.json", summary.id));
        let Ok(file_content) = std::fs::read_to_string(&path) else {
            continue;
        };
        let Ok(account) = crate::modules::secure_account_reader::read_account_detail::<CodexAccount>(
            &path,
            &file_content,
        ) else {
            continue;
        };
        accounts.push(account);
    }
    Ok((accounts, index.current_account_id))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::codex::{CodexAgentIdentity, CodexTokens};
    use crate::models::quota::QuotaData;
    use crate::models::token::TokenData;

    const SECRET_ACCESS: &str = "ya29.SECRET-ACCESS-TOKEN-VALUE";
    const SECRET_REFRESH: &str = "1//REFRESH-SECRET-VALUE";
    const SECRET_ID_TOKEN: &str = "eyJhbGciOiJSUzI1NiJ9.SECRET-PAYLOAD.SECRET-SIG";
    const SECRET_EMAIL: &str = "secret-user@example.com";
    const SECRET_API_KEY: &str = "sk-proj-SECRET-KEY-VALUE";
    const SECRET_SESSION: &str = "SESSION-COOKIE-SECRET";
    const SECRET_AGENT_KEY: &str = "AGENT-PRIVATE-KEY-PEM-BLOB";

    fn fixture_token() -> TokenData {
        TokenData {
            access_token: SECRET_ACCESS.to_string(),
            refresh_token: SECRET_REFRESH.to_string(),
            expires_in: 3600,
            expiry_timestamp: 1_800_000_000,
            token_type: "Bearer".to_string(),
            email: Some(SECRET_EMAIL.to_string()),
            project_id: Some("secret-project".to_string()),
            is_gcp_tos: None,
            oauth_client_key: None,
            id_token: Some(SECRET_ID_TOKEN.to_string()),
            session_id: Some(SECRET_SESSION.to_string()),
        }
    }

    fn fixture_antigravity_account() -> Account {
        let mut account = Account::new(
            "agt-1111-2222".to_string(),
            SECRET_EMAIL.to_string(),
            fixture_token(),
        );
        let mut quota = QuotaData::new();
        quota.add_model(
            "gemini-3-pro".to_string(),
            Some("Gemini 3 Pro".to_string()),
            72,
            "2026-08-31T00:00:00Z".to_string(),
        );
        quota.add_model("claude-sonnet".to_string(), None, 10, String::new());
        quota.subscription_tier = Some("PRO".to_string());
        quota.tier_id = Some("g1-pro-tier".to_string());
        account.quota = Some(quota);
        account.tags = vec!["pool2".to_string()];
        account
    }

    fn fixture_codex_account() -> CodexAccount {
        let mut account = CodexAccount::new(
            "cdx-3333-4444".to_string(),
            SECRET_EMAIL.to_string(),
            CodexTokens {
                id_token: SECRET_ID_TOKEN.to_string(),
                access_token: SECRET_ACCESS.to_string(),
                refresh_token: Some(SECRET_REFRESH.to_string()),
            },
        );
        account.openai_api_key = Some(SECRET_API_KEY.to_string());
        account.agent_identity = Some(CodexAgentIdentity {
            agent_runtime_id: "runtime-1".to_string(),
            agent_private_key: SECRET_AGENT_KEY.to_string(),
            task_id: None,
            account_id: "acct-123".to_string(),
            chatgpt_user_id: "user-abc".to_string(),
            email: Some(SECRET_EMAIL.to_string()),
            plan_type: Some("pro".to_string()),
            chatgpt_account_is_fedramp: false,
        });
        account.plan_type = Some("plus".to_string());
        account.tags = Some(vec!["paid".to_string()]);
        account.usage_updated_at = Some(1_800_000_000);
        account
    }

    fn assert_no_secrets(json: &str) {
        for secret in [
            SECRET_ACCESS,
            SECRET_REFRESH,
            SECRET_ID_TOKEN,
            SECRET_EMAIL,
            SECRET_API_KEY,
            SECRET_SESSION,
            SECRET_AGENT_KEY,
            "secret-project",
            "secret-user",
            "user-abc",
            "acct-123",
            "runtime-1",
        ] {
            assert!(!json.contains(secret), "快照泄漏了敏感值: {}", secret);
        }
    }

    #[test]
    fn alias_is_stable_and_does_not_contain_raw_identity() {
        let alias = account_alias_for("antigravity", "agt-1111-2222");
        assert_eq!(alias, account_alias_for("antigravity", "agt-1111-2222"));
        assert_ne!(account_alias_for("codex", "agt-1111-2222"), alias);
        assert!(!alias.contains("agt-1111"));
        assert!(!alias.contains('@'));
        assert_eq!(
            credential_ref_for("codex", "x"),
            format!("cockpit:codex:{}", opaque_suffix("codex", "x"))
        );
    }

    #[test]
    fn antigravity_route_leaks_no_credentials() {
        let route = route_from_antigravity(&fixture_antigravity_account(), true);
        let json = serde_json::to_string(&route).unwrap();
        assert_no_secrets(&json);
        assert!(route.is_current);
        assert_eq!(route.plan.as_deref(), Some("PRO"));
        assert_eq!(route.quota_windows.len(), 2);
        assert!((route.quota_windows[0].remaining_ratio - 0.72).abs() < 1e-9);
        assert_eq!(
            route.quota_windows[0].reset_at.as_deref(),
            Some("2026-08-31T00:00:00Z")
        );
    }

    #[test]
    fn codex_route_leaks_no_credentials() {
        let route = route_from_codex(&fixture_codex_account(), false);
        let json = serde_json::to_string(&route).unwrap();
        assert_no_secrets(&json);
        assert_eq!(route.provider, "codex");
        assert!(!route.is_current);
        assert_eq!(
            route.metadata.get("auth_mode").and_then(|v| v.as_str()),
            None
        );
    }

    #[test]
    fn codex_route_marks_apikey_mode_without_key_material() {
        let mut account = fixture_codex_account();
        account.auth_mode = CodexAuthMode::Apikey;
        let route = route_from_codex(&account, false);
        let json = serde_json::to_string(&route).unwrap();
        assert_no_secrets(&json);
        assert_eq!(
            route.metadata.get("auth_mode").and_then(|v| v.as_str()),
            Some("apikey")
        );
    }

    #[test]
    fn snapshot_from_parts_leaks_nothing_and_reports_sources() {
        let snapshot = build_snapshot_from_parts(
            &[fixture_antigravity_account()],
            Some("agt-1111-2222"),
            &[fixture_codex_account()],
            None,
        );
        let json = serde_json::to_string_pretty(&snapshot).unwrap();
        assert_no_secrets(&json);
        assert_eq!(snapshot.schema_version, "1.0");
        assert_eq!(snapshot.ttl_seconds, DEFAULT_TTL_SECONDS);
        assert_eq!(snapshot.routes.len(), 2);
        assert!(snapshot.sources.iter().all(|s| s.ok));
        assert_eq!(snapshot.availability, SnapshotAvailability::Ok);
        assert!(serde_json::to_value(&snapshot).unwrap()["routes"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r["is_current"].as_bool() == Some(true)));
    }

    #[test]
    fn quota_window_normalization_handles_unparsable_reset_time() {
        let mut account = fixture_antigravity_account();
        if let Some(quota) = account.quota.as_mut() {
            quota.models[1].reset_time = "not-a-timestamp".to_string();
        }
        let route = route_from_antigravity(&account, false);
        assert_eq!(route.quota_windows[0].reset_at.is_some(), true);
        assert_eq!(route.quota_windows[1].reset_at, None);
    }

    #[test]
    fn codex_windows_respect_window_presence_flags() {
        let mut account = fixture_codex_account();
        account.quota = Some(crate::models::codex::CodexQuota {
            hourly_percentage: 80,
            hourly_reset_time: Some(1_800_000_000),
            hourly_window_minutes: Some(300),
            hourly_window_present: Some(false),
            weekly_percentage: 55,
            weekly_reset_time: Some(1_805_000_000),
            weekly_window_minutes: Some(10_080),
            weekly_window_present: Some(true),
            raw_data: Some(serde_json::json!({ "top_secret_field": SECRET_ACCESS })),
        });
        let route = route_from_codex(&account, false);
        let json = serde_json::to_string(&route).unwrap();
        assert_no_secrets(&json);
        let names: Vec<&str> = route
            .quota_windows
            .iter()
            .map(|w| w.name.as_str())
            .collect();
        assert_eq!(names, vec!["weekly"]);
        assert!((route.quota_windows[0].remaining_ratio - 0.55).abs() < 1e-9);
        assert_eq!(
            route.quota_windows[0].reset_at.as_deref(),
            Some("2027-03-14T04:53:20Z")
        );
    }

    #[test]
    fn health_mapping_covers_disabled_forbidden_auth_and_rate_limit() {
        let mut account = fixture_antigravity_account();
        account.disabled = true;
        assert_eq!(
            health_from(true, false, None).status,
            HealthStatus::Unavailable
        );

        let mut account = fixture_antigravity_account();
        account.quota.as_mut().unwrap().is_forbidden = true;
        let health = health_from(false, true, None);
        assert_eq!(health.status, HealthStatus::Unavailable);
        assert_eq!(health.detail_code.as_deref(), Some("forbidden"));

        let health = health_from(false, false, Some((Some("401"), "Unauthorized", 111)));
        assert_eq!(health.status, HealthStatus::Degraded);
        assert_eq!(health.last_auth_error_at, Some(111));

        let health = health_from(false, false, Some((Some("429"), "rate limit", 222)));
        assert_eq!(health.status, HealthStatus::Degraded);
        assert_eq!(health.last_rate_limit_at, Some(222));

        let health = health_from(false, false, None);
        assert_eq!(health.status, HealthStatus::Healthy);
    }

    #[test]
    fn sanitize_error_code_rejects_freeform_messages() {
        assert_eq!(sanitize_error_code(Some("429")).as_deref(), Some("429"));
        assert_eq!(
            sanitize_error_code(Some("invalid_grant")).as_deref(),
            Some("invalid_grant")
        );
        assert_eq!(sanitize_error_code(Some("Bearer abc def")), None);
        assert_eq!(sanitize_error_code(Some("")), None);
    }

    #[test]
    fn ratio_clamps_out_of_range_percentages() {
        assert!((ratio_from_percentage(-5) - 0.0).abs() < 1e-9);
        assert!((ratio_from_percentage(150) - 1.0).abs() < 1e-9);
    }

    #[test]
    fn availability_is_unavailable_when_every_source_fails() {
        let sources = vec![
            SourceStatus {
                provider: "antigravity".to_string(),
                ok: false,
            },
            SourceStatus {
                provider: "codex".to_string(),
                ok: false,
            },
        ];
        assert_eq!(
            availability_from_sources(&sources),
            SnapshotAvailability::Unavailable
        );
        assert_eq!(
            availability_from_sources(&[]),
            SnapshotAvailability::Unavailable
        );
        let mixed = vec![
            SourceStatus {
                provider: "antigravity".to_string(),
                ok: true,
            },
            SourceStatus {
                provider: "codex".to_string(),
                ok: false,
            },
        ];
        assert_eq!(
            availability_from_sources(&mixed),
            SnapshotAvailability::Degraded
        );
    }

    #[test]
    fn serialized_snapshot_never_contains_secret_shaped_fixture_values() {
        let mut account = fixture_antigravity_account();
        account.quota_error = Some(crate::models::QuotaErrorInfo {
            message: format!("Bearer {SECRET_ACCESS} cookie={SECRET_SESSION}"),
            timestamp: 1_800_000_000,
            code: Some(401),
        });
        let mut codex = fixture_codex_account();
        codex.quota_error = Some(crate::models::codex::CodexQuotaErrorInfo {
            message: format!("refresh={SECRET_REFRESH} key={SECRET_API_KEY}"),
            timestamp: 1_800_000_000,
            code: Some("invalid_grant".to_string()),
        });
        let snapshot = build_snapshot_from_parts(&[account], None, &[codex], None);
        let json = serde_json::to_string_pretty(&snapshot).unwrap();
        assert_no_secrets(&json);
        assert!(json.contains("\"availability\": \"ok\""));
        assert!(!json.to_lowercase().contains("bearer "));
        assert!(!json.contains("cookie="));
    }
}
