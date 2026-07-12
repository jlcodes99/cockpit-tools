//! Grok CLI 多账号管理
//!
//! 兼容官方 Grok Build CLI（Win / macOS / Linux）：
//! - 配置目录：`GROK_HOME` 或 `~/.grok`（Windows 为 `%USERPROFILE%\.grok`）
//! - 登录态：`auth.json`，顶层 key 为 `https://auth.x.ai::<client_id>`
//! - 切号：原子写回官方格式（CLI 会自动热加载，无需重启）
//! - 额度：`https://cli-chat-proxy.grok.com/v1/billing?format=credits` + `/v1/user?include=subscription`
//! - refresh_token 单次有效：刷新后必须同时回写 cockpit 存储与当前 auth.json

use crate::models::grok::{
    GrokAccount, GrokAccountIndex, GrokOAuthStartResponse, GrokQuota,
};
use crate::modules::{account, atomic_write, logger};
use base64::Engine;
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

const ACCOUNTS_INDEX_FILE: &str = "grok_accounts.json";
const ACCOUNTS_DIR: &str = "grok_accounts";
const GROK_HOME_DIR: &str = ".grok";
const AUTH_FILE: &str = "auth.json";
const GROK_HOME_ENV: &str = "GROK_HOME";

/// 官方 Grok CLI OAuth client_id（与 CLIProxyAPI / 本机 auth.json 一致）
pub const GROK_OIDC_CLIENT_ID: &str = "b1a00492-073a-47ea-816f-4c329264a828";
pub const GROK_OIDC_ISSUER: &str = "https://auth.x.ai";
const GROK_OIDC_SCOPE_PREFIX: &str = "https://auth.x.ai::";
const LEGACY_SESSION_SCOPE: &str = "https://accounts.x.ai/sign-in";

const OIDC_DISCOVERY_URL: &str = "https://auth.x.ai/.well-known/openid-configuration";
const OIDC_TOKEN_URL: &str = "https://auth.x.ai/oauth2/token";
const OIDC_USERINFO_URL: &str = "https://auth.x.ai/oauth2/userinfo";
const OIDC_DEVICE_URL: &str = "https://auth.x.ai/oauth2/device/code";
const OIDC_SCOPES: &str =
    "openid profile email offline_access grok-cli:access api:access conversations:read conversations:write";

/// 实时额度（与 grokcli-2api / CLIProxyAPI 一致）
const CLI_CHAT_PROXY_BASE: &str = "https://cli-chat-proxy.grok.com/v1";
/// 与官方 grok-cli / 社区客户端请求头版本对齐（额度接口会校验 surface/version）
const CLI_VERSION: &str = "0.2.97";
const BILLING_URL_PATH: &str = "/billing?format=credits";
const USER_URL_PATH: &str = "/user?include=subscription";
const TOKEN_REFRESH_SKEW_SECONDS: i64 = 5 * 60;

lazy_static::lazy_static! {
    static ref GROK_ACCOUNT_INDEX_LOCK: Mutex<()> = Mutex::new(());
    static ref GROK_TOKEN_REFRESH_LOCKS: Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>> =
        Mutex::new(HashMap::new());
    static ref GROK_OAUTH_SESSIONS: Mutex<HashMap<String, GrokDeviceSession>> =
        Mutex::new(HashMap::new());
}

#[derive(Debug, Clone)]
struct GrokDeviceSession {
    device_code: String,
    verification_uri: String,
    user_code: Option<String>,
    interval_seconds: u64,
    expires_at: i64,
    token_endpoint: String,
}

fn now_ts() -> i64 {
    chrono::Utc::now().timestamp()
}

fn normalize_non_empty(value: Option<&str>) -> Option<String> {
    value.and_then(|raw| {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    })
}

fn get_data_dir() -> Result<PathBuf, String> {
    account::get_data_dir()
}

fn get_accounts_dir() -> Result<PathBuf, String> {
    let dir = get_data_dir()?.join(ACCOUNTS_DIR);
    if !dir.exists() {
        fs::create_dir_all(&dir).map_err(|e| format!("创建 Grok 账号目录失败: {}", e))?;
    }
    Ok(dir)
}

fn get_accounts_index_path() -> Result<PathBuf, String> {
    Ok(get_data_dir()?.join(ACCOUNTS_INDEX_FILE))
}

pub fn accounts_index_path_string() -> Result<String, String> {
    Ok(get_accounts_index_path()?.to_string_lossy().to_string())
}

/// 解析 Grok 配置目录（跨平台）
/// 优先级：`GROK_HOME` > `HOME/.grok`（Windows 下 HOME 对应 USERPROFILE）
pub fn get_grok_home() -> Result<PathBuf, String> {
    if let Ok(raw) = std::env::var(GROK_HOME_ENV) {
        let trimmed = raw.trim().trim_matches('"').trim_matches('\'').trim();
        if !trimmed.is_empty() {
            return Ok(PathBuf::from(trimmed));
        }
    }
    let home = dirs::home_dir().ok_or_else(|| "无法获取用户主目录".to_string())?;
    Ok(home.join(GROK_HOME_DIR))
}

pub fn get_auth_json_path() -> Result<PathBuf, String> {
    Ok(get_grok_home()?.join(AUTH_FILE))
}

fn normalize_account_id(account_id: &str) -> Result<String, String> {
    let trimmed = account_id.trim();
    if trimmed.is_empty() {
        return Err("账号 ID 不能为空".to_string());
    }
    if trimmed.contains('/') || trimmed.contains('\\') || trimmed.contains("..") {
        return Err("账号 ID 非法，包含路径字符".to_string());
    }
    // 允许 auth.x.ai 风格 id（内部以 user_id / uuid 为主）
    let valid = trimmed.chars().all(|ch| {
        ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.' | ':' | '@' | '+')
    });
    if !valid {
        return Err("账号 ID 非法".to_string());
    }
    Ok(trimmed.to_string())
}

fn resolve_account_file_path(account_id: &str) -> Result<PathBuf, String> {
    let normalized = normalize_account_id(account_id)?;
    // 文件名安全：把 `:` 替换掉
    let safe = normalized.replace(':', "_").replace('/', "_");
    Ok(get_accounts_dir()?.join(format!("{}.json", safe)))
}

fn account_id_from_user(user_id: &str) -> String {
    format!("grok_{}", user_id.trim())
}

fn decode_jwt_payload(token: &str) -> Option<Value> {
    let parts: Vec<&str> = token.split('.').collect();
    if parts.len() < 2 {
        return None;
    }
    let payload_b64 = parts[1].replace('-', "+").replace('_', "/");
    let padded = match payload_b64.len() % 4 {
        2 => format!("{}==", payload_b64),
        3 => format!("{}=", payload_b64),
        _ => payload_b64,
    };
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(padded)
        .ok()?;
    serde_json::from_slice(&bytes).ok()
}

fn jwt_claim_string(token: &str, key: &str) -> Option<String> {
    let payload = decode_jwt_payload(token)?;
    normalize_non_empty(payload.get(key).and_then(|v| v.as_str())).or_else(|| {
        payload
            .get(key)
            .and_then(|v| v.as_i64())
            .map(|n| n.to_string())
    })
}

fn jwt_claim_i64(token: &str, key: &str) -> Option<i64> {
    let payload = decode_jwt_payload(token)?;
    payload
        .get(key)
        .and_then(|v| v.as_i64())
        .or_else(|| {
            payload
                .get(key)
                .and_then(|v| v.as_str())
                .and_then(|s| s.parse().ok())
        })
}

/// 将 JWT tier 映射为套餐名（SuperGrok / Heavy 等）
pub fn map_tier_to_plan(tier: Option<i64>) -> (String, String) {
    match tier.unwrap_or(0) {
        0 | 1 => ("Free".into(), "FREE".into()),
        2 => ("SuperGrok Lite".into(), "SUPERGROK_LITE".into()),
        3 => ("SuperGrok".into(), "SUPERGROK".into()),
        4 => ("SuperGrok".into(), "SUPERGROK".into()),
        5 => ("SuperGrok Heavy".into(), "SUPERGROK_HEAVY".into()),
        n if n >= 6 => ("Enterprise".into(), "ENTERPRISE".into()),
        _ => ("Unknown".into(), "UNKNOWN".into()),
    }
}

fn parse_expires_at(value: Option<&Value>, access_token: &str) -> Option<i64> {
    match value {
        Some(Value::Number(n)) => n.as_i64().or_else(|| n.as_f64().map(|f| f as i64)),
        Some(Value::String(s)) => {
            let trimmed = s.trim();
            if trimmed.is_empty() {
                jwt_claim_i64(access_token, "exp")
            } else if let Ok(n) = trimmed.parse::<i64>() {
                Some(n)
            } else if let Ok(f) = trimmed.parse::<f64>() {
                Some(f as i64)
            } else {
                // ISO-8601
                chrono::DateTime::parse_from_rfc3339(trimmed)
                    .ok()
                    .map(|dt| dt.timestamp())
                    .or_else(|| {
                        // 兼容 2026-07-12T06:41:09.232453Z
                        let cleaned = if trimmed.ends_with('Z') {
                            format!("{}+00:00", &trimmed[..trimmed.len() - 1])
                        } else {
                            trimmed.to_string()
                        };
                        // 截断超长小数
                        let cleaned = if let Some((head, rest)) = cleaned.split_once('.') {
                            let mut digits = String::new();
                            let mut tz = String::new();
                            for ch in rest.chars() {
                                if ch.is_ascii_digit() && digits.len() < 6 {
                                    digits.push(ch);
                                } else if !ch.is_ascii_digit() {
                                    tz.push(ch);
                                    // 剩余都是时区
                                    for c2 in rest.chars().skip(digits.len() + tz.len() - 1).skip(1)
                                    {
                                        tz.push(c2);
                                    }
                                    break;
                                }
                            }
                            while digits.len() < 6 {
                                digits.push('0');
                            }
                            format!("{}.{}{}", head, &digits[..6], tz)
                        } else {
                            cleaned
                        };
                        chrono::DateTime::parse_from_rfc3339(&cleaned)
                            .ok()
                            .map(|dt| dt.timestamp())
                    })
                    .or_else(|| jwt_claim_i64(access_token, "exp"))
            }
        }
        _ => jwt_claim_i64(access_token, "exp"),
    }
}

fn expires_at_to_iso(ts: i64) -> String {
    chrono::DateTime::<chrono::Utc>::from_timestamp(ts, 0)
        .map(|dt| dt.format("%Y-%m-%dT%H:%M:%S%.6fZ").to_string())
        .unwrap_or_else(|| format!("{}Z", ts))
}

fn money_val(obj: Option<&Value>) -> Option<f64> {
    match obj {
        Some(Value::Number(n)) => n.as_f64(),
        Some(Value::Object(map)) => map.get("val").and_then(|v| {
            v.as_f64()
                .or_else(|| v.as_i64().map(|i| i as f64))
                .or_else(|| v.as_str()?.parse().ok())
        }),
        Some(Value::String(s)) => s.trim().parse().ok(),
        _ => None,
    }
}

fn load_index() -> GrokAccountIndex {
    let Ok(path) = get_accounts_index_path() else {
        return GrokAccountIndex::new();
    };
    if !path.exists() {
        return GrokAccountIndex::new();
    }
    match fs::read_to_string(&path) {
        Ok(content) => atomic_write::parse_json_with_auto_restore(&path, &content)
            .unwrap_or_else(|_| GrokAccountIndex::new()),
        Err(_) => GrokAccountIndex::new(),
    }
}

fn save_index(index: &GrokAccountIndex) -> Result<(), String> {
    let _guard = GROK_ACCOUNT_INDEX_LOCK
        .lock()
        .map_err(|_| "Grok 账号索引锁异常".to_string())?;
    let path = get_accounts_index_path()?;
    let content =
        serde_json::to_string_pretty(index).map_err(|e| format!("序列化索引失败: {}", e))?;
    atomic_write::write_string_atomic(&path, &content)
        .map_err(|e| format!("写入 Grok 账号索引失败: {}", e))
}

pub fn load_account(account_id: &str) -> Option<GrokAccount> {
    let path = resolve_account_file_path(account_id).ok()?;
    if !path.exists() {
        return None;
    }
    let content = fs::read_to_string(&path).ok()?;
    atomic_write::parse_json_with_auto_restore(&path, &content).ok()
}

fn save_account(account: &GrokAccount) -> Result<(), String> {
    let path = resolve_account_file_path(&account.id)?;
    let content =
        serde_json::to_string_pretty(account).map_err(|e| format!("序列化账号失败: {}", e))?;
    atomic_write::write_string_atomic(&path, &content)
        .map_err(|e| format!("写入 Grok 账号失败: {}", e))?;

    // 索引 load+merge+save 同锁，避免并发丢项
    let _guard = GROK_ACCOUNT_INDEX_LOCK
        .lock()
        .map_err(|_| "Grok 账号索引锁异常".to_string())?;
    let index_path = get_accounts_index_path()?;
    let mut index = if index_path.exists() {
        fs::read_to_string(&index_path)
            .ok()
            .and_then(|c| atomic_write::parse_json_with_auto_restore(&index_path, &c).ok())
            .unwrap_or_else(GrokAccountIndex::new)
    } else {
        GrokAccountIndex::new()
    };
    let summary = account.summary();
    if let Some(pos) = index.accounts.iter().position(|a| a.id == account.id) {
        index.accounts[pos] = summary;
    } else {
        index.accounts.push(summary);
    }
    let index_content =
        serde_json::to_string_pretty(&index).map_err(|e| format!("序列化索引失败: {}", e))?;
    atomic_write::write_string_atomic(&index_path, &index_content)
        .map_err(|e| format!("写入 Grok 账号索引失败: {}", e))
}

fn delete_account_file(account_id: &str) -> Result<(), String> {
    let path = resolve_account_file_path(account_id)?;
    if path.exists() {
        fs::remove_file(&path).map_err(|e| format!("删除账号文件失败: {}", e))?;
    }
    Ok(())
}

fn enrich_from_token(account: &mut GrokAccount) {
    let token = account.access_token.as_str();
    if account.user_id.is_none() {
        account.user_id = jwt_claim_string(token, "sub")
            .or_else(|| jwt_claim_string(token, "principal_id"));
    }
    if account.principal_id.is_none() {
        account.principal_id = jwt_claim_string(token, "principal_id")
            .or_else(|| account.user_id.clone());
    }
    if account.team_id.is_none() {
        account.team_id = jwt_claim_string(token, "team_id");
    }
    if account.tier.is_none() {
        account.tier = jwt_claim_i64(token, "tier");
    }
    if account.scope.is_none() {
        account.scope = jwt_claim_string(token, "scope");
    }
    if account.expires_at.is_none() {
        account.expires_at = jwt_claim_i64(token, "exp");
    }
    if let Some(tier) = account.tier {
        let (label, plan) = map_tier_to_plan(Some(tier));
        account.plan_label = Some(label);
        account.plan_type = Some(plan);
    }
    if account.oidc_client_id.is_none() {
        account.oidc_client_id = jwt_claim_string(token, "client_id")
            .or_else(|| jwt_claim_string(token, "aud"))
            .or_else(|| Some(GROK_OIDC_CLIENT_ID.to_string()));
    }
    if account.oidc_issuer.is_none() {
        account.oidc_issuer = jwt_claim_string(token, "iss")
            .or_else(|| Some(GROK_OIDC_ISSUER.to_string()));
    }
}

/// 从 auth.json 单条 entry 构造账号
fn account_from_auth_entry(
    entry_key: &str,
    entry: &Map<String, Value>,
    existing: Option<GrokAccount>,
) -> Result<GrokAccount, String> {
    let access_token = normalize_non_empty(
        entry
            .get("key")
            .or_else(|| entry.get("access_token"))
            .or_else(|| entry.get("token"))
            .and_then(|v| v.as_str()),
    )
    .ok_or_else(|| format!("auth entry {} 缺少 access token", entry_key))?;

    let refresh_token = normalize_non_empty(entry.get("refresh_token").and_then(|v| v.as_str()));
    let expires_at = parse_expires_at(entry.get("expires_at"), &access_token);
    let expires_at_raw = entry
        .get("expires_at")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let user_id = normalize_non_empty(entry.get("user_id").and_then(|v| v.as_str()))
        .or_else(|| normalize_non_empty(entry.get("principal_id").and_then(|v| v.as_str())))
        .or_else(|| jwt_claim_string(&access_token, "sub"))
        .or_else(|| jwt_claim_string(&access_token, "principal_id"));

    let email = normalize_non_empty(entry.get("email").and_then(|v| v.as_str()))
        .or_else(|| jwt_claim_string(&access_token, "email"))
        .unwrap_or_else(|| {
            user_id
                .clone()
                .map(|u| format!("{}@x.ai", u))
                .unwrap_or_else(|| "unknown@x.ai".into())
        });

    let first_name = normalize_non_empty(entry.get("first_name").and_then(|v| v.as_str()));
    let last_name = normalize_non_empty(entry.get("last_name").and_then(|v| v.as_str()));
    let name = match (&first_name, &last_name) {
        (Some(f), Some(l)) => Some(format!("{} {}", f, l).trim().to_string()),
        (Some(f), None) => Some(f.clone()),
        (None, Some(l)) => Some(l.clone()),
        _ => None,
    };

    let id = existing
        .as_ref()
        .map(|a| a.id.clone())
        .or_else(|| user_id.as_ref().map(|u| account_id_from_user(u)))
        .unwrap_or_else(|| {
            format!(
                "grok_{}",
                uuid::Uuid::new_v4().to_string().replace('-', "")
            )
        });

    let now = now_ts();
    let mut account = GrokAccount {
        id,
        email,
        name,
        first_name,
        last_name,
        user_id,
        principal_id: normalize_non_empty(entry.get("principal_id").and_then(|v| v.as_str())),
        team_id: normalize_non_empty(entry.get("team_id").and_then(|v| v.as_str())),
        profile_image_asset_id: normalize_non_empty(
            entry.get("profile_image_asset_id").and_then(|v| v.as_str()),
        ),
        tier: jwt_claim_i64(&access_token, "tier"),
        plan_type: None,
        plan_label: None,
        access_token,
        refresh_token,
        scope: None,
        expires_at,
        expires_at_raw,
        oidc_issuer: normalize_non_empty(entry.get("oidc_issuer").and_then(|v| v.as_str()))
            .or_else(|| Some(GROK_OIDC_ISSUER.to_string())),
        oidc_client_id: normalize_non_empty(entry.get("oidc_client_id").and_then(|v| v.as_str()))
            .or_else(|| Some(GROK_OIDC_CLIENT_ID.to_string())),
        auth_entry_key: Some(entry_key.to_string()),
        auth_mode_raw: normalize_non_empty(entry.get("auth_mode").and_then(|v| v.as_str()))
            .or_else(|| Some("oidc".into())),
        create_time: normalize_non_empty(entry.get("create_time").and_then(|v| v.as_str())),
        coding_data_retention_opt_out: entry
            .get("coding_data_retention_opt_out")
            .and_then(|v| v.as_bool()),
        has_grok_code_access: existing.as_ref().and_then(|a| a.has_grok_code_access),
        quota: existing.as_ref().and_then(|a| a.quota.clone()),
        usage_updated_at: existing.as_ref().and_then(|a| a.usage_updated_at),
        token_updated_at: Some(now),
        status: Some("active".into()),
        status_reason: None,
        requires_reauth: Some(false),
        reauth_reason: None,
        quota_query_last_error: None,
        quota_query_last_error_at: None,
        subscription_query_last_success_at: existing
            .as_ref()
            .and_then(|a| a.subscription_query_last_success_at),
        auth_raw: Some(Value::Object(entry.clone())),
        userinfo_raw: existing.as_ref().and_then(|a| a.userinfo_raw.clone()),
        billing_raw: existing.as_ref().and_then(|a| a.billing_raw.clone()),
        user_raw: existing.as_ref().and_then(|a| a.user_raw.clone()),
        tags: existing.as_ref().and_then(|a| a.tags.clone()),
        account_note: existing.as_ref().and_then(|a| a.account_note.clone()),
        created_at: existing.as_ref().map(|a| a.created_at).unwrap_or(now),
        last_used: existing.as_ref().map(|a| a.last_used).unwrap_or(now),
    };
    enrich_from_token(&mut account);
    Ok(account)
}

/// 选择 auth.json 中优先条目（官方 OIDC > legacy session > 最新 expires）
fn select_preferred_entries(root: &Map<String, Value>) -> Vec<(String, Map<String, Value>)> {
    let mut entries: Vec<(String, Map<String, Value>, i64)> = Vec::new();
    for (key, value) in root.iter() {
        let Some(obj) = value.as_object() else {
            continue;
        };
        let token = obj
            .get("key")
            .or_else(|| obj.get("access_token"))
            .and_then(|v| v.as_str())
            .unwrap_or("");
        if token.is_empty() {
            continue;
        }
        let exp = parse_expires_at(obj.get("expires_at"), token).unwrap_or(0);
        entries.push((key.clone(), obj.clone(), exp));
    }
    entries.sort_by(|a, b| {
        let score = |k: &str| {
            if k.starts_with(GROK_OIDC_SCOPE_PREFIX) {
                2
            } else if k == LEGACY_SESSION_SCOPE {
                1
            } else {
                0
            }
        };
        score(&b.0)
            .cmp(&score(&a.0))
            .then_with(|| b.2.cmp(&a.2))
    });
    entries
        .into_iter()
        .map(|(k, v, _)| (k, v))
        .collect()
}

pub fn list_accounts() -> Vec<GrokAccount> {
    let index = load_index();
    let mut accounts = Vec::new();
    for summary in index.accounts {
        if let Some(account) = load_account(&summary.id) {
            accounts.push(account);
        }
    }
    accounts.sort_by(|a, b| b.last_used.cmp(&a.last_used));
    accounts
}

pub fn list_accounts_checked() -> Result<Vec<GrokAccount>, String> {
    Ok(list_accounts())
}

pub fn resolve_current_account_id() -> Option<String> {
    let index = load_index();
    if let Some(id) = index.current_account_id {
        if load_account(&id).is_some() {
            return Some(id);
        }
    }
    // 回退：与本地 auth.json 匹配
    if let Ok(Some(local)) = peek_local_auth_account() {
        let accounts = list_accounts();
        if let Some(found) = accounts.iter().find(|a| {
            a.user_id == local.user_id
                || (!a.email.is_empty() && a.email == local.email)
                || a.access_token == local.access_token
        }) {
            return Some(found.id.clone());
        }
    }
    None
}

pub fn set_current_account_id(account_id: Option<&str>) -> Result<(), String> {
    let mut index = load_index();
    index.current_account_id = account_id.map(|s| s.to_string());
    save_index(&index)
}

pub fn get_current_account() -> Option<GrokAccount> {
    resolve_current_account_id().and_then(|id| load_account(&id))
}

fn peek_local_auth_account() -> Result<Option<GrokAccount>, String> {
    let path = get_auth_json_path()?;
    if !path.exists() {
        return Ok(None);
    }
    let content =
        fs::read_to_string(&path).map_err(|e| format!("读取 auth.json 失败: {}", e))?;
    let root: Value =
        serde_json::from_str(&content).map_err(|e| format!("解析 auth.json 失败: {}", e))?;
    let Some(map) = root.as_object() else {
        return Ok(None);
    };
    let entries = select_preferred_entries(map);
    let Some((key, entry)) = entries.into_iter().next() else {
        return Ok(None);
    };
    Ok(Some(account_from_auth_entry(&key, &entry, None)?))
}

/// 从本机 `~/.grok/auth.json` 导入（可含多条，官方 CLI 通常 1 条）
pub fn import_from_local() -> Result<Vec<GrokAccount>, String> {
    let path = get_auth_json_path()?;
    if !path.exists() {
        return Err(format!(
            "未找到本地 Grok 登录信息: {}（请先运行 grok login）",
            path.display()
        ));
    }
    let content =
        fs::read_to_string(&path).map_err(|e| format!("读取 auth.json 失败: {}", e))?;
    import_from_json(&content)
}

pub fn import_from_json(json_content: &str) -> Result<Vec<GrokAccount>, String> {
    let root: Value =
        serde_json::from_str(json_content).map_err(|e| format!("JSON 解析失败: {}", e))?;

    let mut imported = Vec::new();

    // 支持：1) 官方 auth.json map  2) 单条 entry  3) 账号数组  4) cockpit 导出数组
    if let Some(map) = root.as_object() {
        // 判断是否为 auth.json 风格
        let looks_like_auth_map = map.values().any(|v| {
            v.as_object()
                .map(|o| {
                    o.contains_key("key")
                        || o.contains_key("access_token")
                        || o.contains_key("refresh_token")
                })
                .unwrap_or(false)
        });

        if looks_like_auth_map
            && !map.contains_key("access_token")
            && !map.contains_key("email")
            && !map.contains_key("id")
        {
            for (key, value) in select_preferred_entries(map) {
                // 官方 CLI 单槽 key 固定为 issuer::client_id，不能用来匹配身份，否则会错覆盖多账号
                let entry_uid = value
                    .get("user_id")
                    .and_then(|v| v.as_str())
                    .or_else(|| value.get("principal_id").and_then(|v| v.as_str()))
                    .map(|s| s.to_string())
                    .or_else(|| {
                        let tok = value
                            .get("key")
                            .or_else(|| value.get("access_token"))
                            .and_then(|v| v.as_str())
                            .unwrap_or("");
                        jwt_claim_string(tok, "sub")
                            .or_else(|| jwt_claim_string(tok, "principal_id"))
                    });
                let entry_email = value
                    .get("email")
                    .and_then(|v| v.as_str())
                    .map(|s| s.trim().to_lowercase())
                    .filter(|s| !s.is_empty() && !s.starts_with("unknown@"));
                let existing = list_accounts().into_iter().find(|a| {
                    if let (Some(au), Some(eu)) = (a.user_id.as_deref(), entry_uid.as_deref()) {
                        if au == eu {
                            return true;
                        }
                    }
                    if let (Some(ae), Some(ee)) = (
                        a.email
                            .as_str()
                            .map(|s| s.trim().to_lowercase())
                            .filter(|s| !s.is_empty() && !s.starts_with("unknown@")),
                        entry_email.as_ref(),
                    ) {
                        return ae == *ee;
                    }
                    false
                });
                let account = account_from_auth_entry(&key, &value, existing)?;
                save_account(&account)?;
                imported.push(account);
            }
        } else {
            // 单账号对象
            let entry = map.clone();
            let key = map
                .get("auth_entry_key")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
                .unwrap_or_else(|| {
                    let cid = map
                        .get("oidc_client_id")
                        .and_then(|v| v.as_str())
                        .unwrap_or(GROK_OIDC_CLIENT_ID);
                    format!("{}{}", GROK_OIDC_SCOPE_PREFIX, cid)
                });
            // 兼容 cockpit 账号结构
            if map.get("access_token").and_then(|v| v.as_str()).is_some()
                || map.get("key").and_then(|v| v.as_str()).is_some()
            {
                let mut entry_obj = Map::new();
                for (k, v) in map.iter() {
                    entry_obj.insert(k.clone(), v.clone());
                }
                if !entry_obj.contains_key("key") {
                    if let Some(at) = entry_obj.get("access_token").cloned() {
                        entry_obj.insert("key".into(), at);
                    }
                }
                let existing = map
                    .get("id")
                    .and_then(|v| v.as_str())
                    .and_then(load_account);
                let account = account_from_auth_entry(&key, &entry_obj, existing)?;
                save_account(&account)?;
                imported.push(account);
            } else {
                let _ = entry;
                return Err("无法识别的 Grok JSON 格式".into());
            }
        }
    } else if let Some(arr) = root.as_array() {
        for item in arr {
            let text = serde_json::to_string(item)
                .map_err(|e| format!("序列化导入项失败: {}", e))?;
            let mut part = import_from_json(&text)?;
            imported.append(&mut part);
        }
    } else {
        return Err("JSON 必须是对象或数组".into());
    }

    if imported.is_empty() {
        return Err("未导入任何有效 Grok 账号".into());
    }
    Ok(imported)
}

pub fn export_accounts(account_ids: &[String]) -> Result<String, String> {
    let ids: Vec<String> = if account_ids.is_empty() {
        list_accounts().into_iter().map(|a| a.id).collect()
    } else {
        account_ids.to_vec()
    };
    if ids.is_empty() {
        return Err("没有可导出的 Grok 账号".into());
    }

    // 导出为官方 auth.json 结构，便于备份后直接还原/导入
    // 多账号时使用 user 维度 key，避免 client_id 单槽互相覆盖
    let mut auth_map = Map::new();
    for id in ids {
        let account = load_account(&id).ok_or_else(|| format!("账号不存在: {}", id))?;
        let (key, entry) = build_auth_entry_for_export(&account);
        // 若 key 冲突，附加 user_id 后缀保证可还原
        let final_key = if auth_map.contains_key(&key) {
            account
                .user_id
                .as_ref()
                .map(|u| format!("{}{}", GROK_OIDC_SCOPE_PREFIX, u))
                .unwrap_or_else(|| format!("{}{}", key, uuid::Uuid::new_v4()))
        } else {
            key
        };
        auth_map.insert(final_key, Value::Object(entry));
    }
    serde_json::to_string_pretty(&Value::Object(auth_map))
        .map_err(|e| format!("导出失败: {}", e))
}

/// 导出用 entry key：优先 user_id 维度，便于多账号备份共存
fn build_auth_entry_for_export(account: &GrokAccount) -> (String, Map<String, Value>) {
    let (_, entry) = build_auth_entry(account);
    if let Some(uid) = account.user_id.as_ref().filter(|s| !s.is_empty()) {
        return (format!("{}{}", GROK_OIDC_SCOPE_PREFIX, uid), entry);
    }
    build_auth_entry(account)
}

/// 构造官方 CLI 可识别的 auth.json entry
///
/// 字段以本机 `~/.grok/auth.json` 实测为准（2026 官方 OIDC 槽）：
/// `key`(access), `refresh_token`, `expires_at`, `auth_mode`, `create_time`,
/// `user_id`, `email`, `first_name`, `last_name`, `principal_id`, `principal_type`,
/// `team_id`, `coding_data_retention_opt_out`, `oidc_issuer`, `oidc_client_id`,
/// 可选 `profile_image_asset_id`。
///
/// 写入策略：先保留 `auth_raw` 中未知/未来字段，再用账号当前凭据覆盖官方已知字段，
/// 避免切号时丢字段或把旧 token 写回。
fn build_auth_entry(account: &GrokAccount) -> (String, Map<String, Value>) {
    let client_id = account
        .oidc_client_id
        .as_deref()
        .filter(|s| !s.is_empty())
        .unwrap_or(GROK_OIDC_CLIENT_ID);
    // 官方 Grok CLI 单槽 key：issuer::client_id（切号只替换此槽，不改 map 结构名）
    let key = format!("{}{}", GROK_OIDC_SCOPE_PREFIX, client_id);

    // 1) 以导入时的原始 entry 为底，保留官方可能新增的未知字段
    let mut entry = match &account.auth_raw {
        Some(Value::Object(raw)) => raw.clone(),
        _ => Map::new(),
    };
    // 官方 access 字段名是 `key`，不是 access_token；清掉易混淆别名
    entry.remove("access_token");
    entry.remove("token");

    // 2) 用账号当前态覆盖已知字段（token 刷新后必须用新值）
    entry.insert("key".into(), Value::String(account.access_token.clone()));
    entry.insert(
        "auth_mode".into(),
        Value::String(
            account
                .auth_mode_raw
                .clone()
                .unwrap_or_else(|| "oidc".into()),
        ),
    );
    if let Some(v) = &account.create_time {
        entry.insert("create_time".into(), Value::String(v.clone()));
    } else if !entry.contains_key("create_time") {
        // 仅当原始 entry 也没有时才补，避免无意义改写 create_time
        entry.insert(
            "create_time".into(),
            Value::String(
                chrono::Utc::now()
                    .format("%Y-%m-%dT%H:%M:%S%.6fZ")
                    .to_string(),
            ),
        );
    }
    if let Some(v) = &account.user_id {
        entry.insert("user_id".into(), Value::String(v.clone()));
    }
    if !account.email.is_empty() && !account.email.starts_with("unknown@") {
        entry.insert("email".into(), Value::String(account.email.clone()));
    }
    if let Some(v) = &account.first_name {
        entry.insert("first_name".into(), Value::String(v.clone()));
    }
    if let Some(v) = &account.last_name {
        entry.insert("last_name".into(), Value::String(v.clone()));
    }
    if let Some(v) = &account.profile_image_asset_id {
        entry.insert("profile_image_asset_id".into(), Value::String(v.clone()));
    }
    // 本机官方文件固定 principal_type = "User"
    if !entry.contains_key("principal_type") {
        entry.insert("principal_type".into(), Value::String("User".into()));
    }
    if let Some(v) = account.principal_id.as_ref().or(account.user_id.as_ref()) {
        entry.insert("principal_id".into(), Value::String(v.clone()));
    }
    if let Some(v) = &account.team_id {
        entry.insert("team_id".into(), Value::String(v.clone()));
    }
    if let Some(flag) = account.coding_data_retention_opt_out {
        entry.insert("coding_data_retention_opt_out".into(), Value::Bool(flag));
    } else if !entry.contains_key("coding_data_retention_opt_out") {
        entry.insert("coding_data_retention_opt_out".into(), Value::Bool(false));
    }
    if let Some(v) = &account.refresh_token {
        entry.insert("refresh_token".into(), Value::String(v.clone()));
    }
    // 优先保留官方原始 expires_at 字符串（如 2026-07-12T09:37:17.769Z），避免改写小数位
    if let Some(raw) = account
        .expires_at_raw
        .as_ref()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
    {
        entry.insert("expires_at".into(), Value::String(raw.to_string()));
    } else if let Some(ts) = account.expires_at {
        entry.insert("expires_at".into(), Value::String(expires_at_to_iso(ts)));
    }
    entry.insert(
        "oidc_issuer".into(),
        Value::String(
            account
                .oidc_issuer
                .clone()
                .unwrap_or_else(|| GROK_OIDC_ISSUER.into()),
        ),
    );
    entry.insert(
        "oidc_client_id".into(),
        Value::String(client_id.to_string()),
    );
    (key, entry)
}

/// 切号：原子写回 `~/.grok/auth.json`（官方格式，跨平台）
pub fn inject_account(account_id: &str) -> Result<GrokAccount, String> {
    let mut account =
        load_account(account_id).ok_or_else(|| format!("Grok 账号不存在: {}", account_id))?;
    if account.access_token.trim().is_empty() {
        return Err("账号缺少 access_token".into());
    }
    if account.refresh_token.as_deref().unwrap_or("").is_empty() {
        logger::log_warn(&format!(
            "[Grok Switch] 账号无 refresh_token，切号后无法自动续期: {}",
            account.email
        ));
    }

    write_auth_json_for_account(&account)?;

    account.update_last_used();
    account.status = Some("active".into());
    account.status_reason = None;
    // 与刚写入的 entry 对齐，避免 cockpit 内 auth_raw 仍是旧 token
    let (_, written) = build_auth_entry(&account);
    account.auth_raw = Some(Value::Object(written));
    save_account(&account)?;
    set_current_account_id(Some(&account.id))?;

    // 同步 provider_current_state
    let _ = crate::modules::provider_current_state::set_current_account_id(
        "grok",
        Some(account.id.as_str()),
    );

    logger::log_info(&format!(
        "[Grok Switch] 切号成功: account_id={}, email={}, auth={}",
        account.id,
        account.email,
        get_auth_json_path()
            .map(|p| p.display().to_string())
            .unwrap_or_default()
    ));
    Ok(account)
}

fn write_auth_json_for_account(account: &GrokAccount) -> Result<(), String> {
    let grok_home = get_grok_home()?;
    if !grok_home.exists() {
        fs::create_dir_all(&grok_home).map_err(|e| {
            format!(
                "创建 Grok 目录失败: path={}, error={}",
                grok_home.display(),
                e
            )
        })?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = fs::set_permissions(&grok_home, fs::Permissions::from_mode(0o700));
        }
    }

    let auth_path = grok_home.join(AUTH_FILE);
    // 备份旧文件
    if auth_path.exists() {
        let bak = auth_path.with_extension(format!("json.bak.{}", now_ts()));
        let _ = fs::copy(&auth_path, &bak);
    }

    let (key, entry) = build_auth_entry(account);
    // 合并写入：只替换官方 OIDC 槽，保留其它无关 key（legacy / 手工条目）
    let mut root_map = if auth_path.exists() {
        fs::read_to_string(&auth_path)
            .ok()
            .and_then(|c| serde_json::from_str::<Value>(&c).ok())
            .and_then(|v| v.as_object().cloned())
            .unwrap_or_default()
    } else {
        Map::new()
    };
    root_map.insert(key, Value::Object(entry));
    let content = serde_json::to_string_pretty(&Value::Object(root_map))
        .map_err(|e| format!("序列化 auth.json 失败: {}", e))?;
    atomic_write::write_string_atomic(&auth_path, &format!("{}\n", content))
        .map_err(|e| format!("写入 auth.json 失败: {}", e))?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(&auth_path, fs::Permissions::from_mode(0o600));
    }

    Ok(())
}

pub fn remove_account(account_id: &str) -> Result<(), String> {
    let was_current = resolve_current_account_id().as_deref() == Some(account_id);
    delete_account_file(account_id)?;
    let mut index = load_index();
    index.accounts.retain(|a| a.id != account_id);
    if index.current_account_id.as_deref() == Some(account_id) {
        index.current_account_id = None;
    }
    save_index(&index)?;
    if was_current {
        let _ = crate::modules::provider_current_state::set_current_account_id("grok", None);
        // 备份并清空 CLI 登录态，避免 CLI 仍使用已删账号
        if let Ok(auth_path) = get_auth_json_path() {
            if auth_path.exists() {
                let bak = auth_path.with_extension(format!("json.bak.{}", now_ts()));
                let _ = fs::copy(&auth_path, &bak);
                let empty = "{}\n";
                let _ = atomic_write::write_string_atomic(&auth_path, empty);
            }
        }
    }
    Ok(())
}

pub fn remove_accounts(account_ids: &[String]) -> Result<(), String> {
    for id in account_ids {
        remove_account(id)?;
    }
    Ok(())
}

pub fn update_account_tags(account_id: &str, tags: Vec<String>) -> Result<GrokAccount, String> {
    let mut account =
        load_account(account_id).ok_or_else(|| format!("账号不存在: {}", account_id))?;
    let cleaned: Vec<String> = tags
        .into_iter()
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty())
        .collect();
    account.tags = if cleaned.is_empty() {
        None
    } else {
        Some(cleaned)
    };
    save_account(&account)?;
    Ok(account)
}

pub fn set_account_status(
    account_id: &str,
    status: Option<&str>,
    reason: Option<&str>,
) -> Result<(), String> {
    let mut account =
        load_account(account_id).ok_or_else(|| format!("账号不存在: {}", account_id))?;
    account.status = status.map(|s| s.to_string());
    account.status_reason = reason.map(|s| s.to_string());
    save_account(&account)
}

fn token_refresh_lock(account_id: &str) -> Arc<tokio::sync::Mutex<()>> {
    let mut map = GROK_TOKEN_REFRESH_LOCKS
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    map.entry(account_id.to_string())
        .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
        .clone()
}

pub fn needs_token_refresh(account: &GrokAccount) -> bool {
    let Some(exp) = account.expires_at else {
        // 无明确过期时看 JWT
        if let Some(jwt_exp) = jwt_claim_i64(&account.access_token, "exp") {
            return now_ts() >= jwt_exp - TOKEN_REFRESH_SKEW_SECONDS;
        }
        return false;
    };
    now_ts() >= exp - TOKEN_REFRESH_SKEW_SECONDS
}

async fn refresh_access_token_http(
    refresh_token: &str,
    client_id: &str,
) -> Result<(String, Option<String>, Option<i64>, Option<String>), String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|e| format!("创建 HTTP 客户端失败: {}", e))?;

    let form = [
        ("grant_type", "refresh_token"),
        ("refresh_token", refresh_token),
        ("client_id", client_id),
    ];

    let resp = client
        .post(OIDC_TOKEN_URL)
        .header(CONTENT_TYPE, "application/x-www-form-urlencoded")
        .header("Accept", "application/json")
        .form(&form)
        .send()
        .await
        .map_err(|e| format!("刷新 token 请求失败: {}", e))?;

    let status = resp.status();
    let text = resp
        .text()
        .await
        .map_err(|e| format!("读取刷新响应失败: {}", e))?;
    if !status.is_success() {
        return Err(format!("刷新 token 失败 HTTP {}: {}", status.as_u16(), text));
    }

    let body: Value =
        serde_json::from_str(&text).map_err(|e| format!("解析刷新响应失败: {}", e))?;
    let access = body
        .get("access_token")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "刷新响应缺少 access_token".to_string())?
        .to_string();
    let new_refresh = body
        .get("refresh_token")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let expires_in = body.get("expires_in").and_then(|v| v.as_i64());
    let scope = body
        .get("scope")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    Ok((access, new_refresh, expires_in, scope))
}

fn cli_proxy_headers(token: &str) -> reqwest::header::HeaderMap {
    let mut headers = reqwest::header::HeaderMap::new();
    if let Ok(v) = reqwest::header::HeaderValue::from_str(&format!("Bearer {}", token)) {
        headers.insert(AUTHORIZATION, v);
    }
    if let Ok(v) = reqwest::header::HeaderValue::from_static("application/json") {
        headers.insert(reqwest::header::ACCEPT, v);
    }
    if let Ok(v) = reqwest::header::HeaderValue::from_str(CLI_VERSION) {
        headers.insert("x-grok-client-version", v);
    }
    if let Ok(v) = reqwest::header::HeaderValue::from_static("grok-cli") {
        headers.insert("x-grok-client-surface", v);
    }
    if let Ok(v) = reqwest::header::HeaderValue::from_static("cockpit-tools") {
        headers.insert("x-grok-client-identifier", v);
    }
    if let Ok(v) =
        reqwest::header::HeaderValue::from_str(&format!("grok-cli/{}", CLI_VERSION))
    {
        headers.insert(reqwest::header::USER_AGENT, v);
    }
    headers
}

/// 从 credits bag / 任意对象里取 total/used/remaining
fn credits_bag_usage(bag: &Value) -> Option<(f64, f64)> {
    if !bag.is_object() {
        return None;
    }
    let total = money_val(
        bag.get("total")
            .or_else(|| bag.get("limit"))
            .or_else(|| bag.get("cap"))
            .or_else(|| bag.get("allocation"))
            .or_else(|| bag.get("amount")),
    );
    let remaining = money_val(
        bag.get("remaining")
            .or_else(|| bag.get("balance"))
            .or_else(|| bag.get("left")),
    );
    let used = money_val(
        bag.get("used")
            .or_else(|| bag.get("spent"))
            .or_else(|| bag.get("consumed")),
    );
    if let Some(t) = total {
        if t > 0.0 {
            let u = used.unwrap_or_else(|| {
                remaining
                    .map(|r| (t - r).max(0.0))
                    .unwrap_or(0.0)
            });
            return Some((t, u.max(0.0)));
        }
    }
    if let Some(r) = remaining {
        if r >= 0.0 {
            // 仅知剩余：把 remaining 当 total、used=0，便于展示剩余百分比
            let t = if r > 0.0 { r } else { 1.0 };
            let u = if r > 0.0 { 0.0 } else { 1.0 };
            return Some((t, u));
        }
    }
    None
}

fn period_str(cfg: &Value, keys: &[&str]) -> Option<String> {
    for k in keys {
        if let Some(s) = cfg.get(*k).and_then(|v| v.as_str()) {
            if !s.is_empty() {
                return Some(s.to_string());
            }
        }
    }
    // currentPeriod: { start, end }
    if let Some(period) = cfg.get("currentPeriod").and_then(|v| v.as_object()) {
        for k in keys {
            // keys may be billingPeriodEnd -> try "end"
            let alt = if k.contains("End") || k.ends_with("end") {
                "end"
            } else if k.contains("Start") || k.ends_with("start") {
                "start"
            } else {
                continue;
            };
            if let Some(s) = period.get(alt).and_then(|v| v.as_str()) {
                if !s.is_empty() {
                    return Some(s.to_string());
                }
            }
        }
    }
    None
}

fn normalize_billing(raw: &Value) -> GrokQuota {
    let cfg = raw
        .get("config")
        .filter(|v| v.is_object())
        .unwrap_or(raw);

    let mut monthly_limit = money_val(cfg.get("monthlyLimit").or_else(|| cfg.get("monthly_limit")));
    let mut used = money_val(cfg.get("used"));
    let on_demand_cap = money_val(cfg.get("onDemandCap").or_else(|| cfg.get("on_demand_cap")));
    let on_demand_used =
        money_val(cfg.get("onDemandUsed").or_else(|| cfg.get("on_demand_used")));
    let prepaid =
        money_val(cfg.get("prepaidBalance").or_else(|| cfg.get("prepaid_balance")));

    // format=credits：统一账期常以 onDemand / credits bag 为主（无 monthlyLimit）
    if monthly_limit.is_none() || monthly_limit == Some(0.0) {
        if let Some(cap) = on_demand_cap {
            if cap > 0.0 {
                monthly_limit = Some(cap);
                used = Some(on_demand_used.unwrap_or(0.0).max(0.0));
            } else if cap == 0.0 {
                // cap=0 常表示免费/促销额度已尽（chat 可能 402 spending-limit）
                monthly_limit = Some(1.0);
                used = Some(1.0);
            }
        }
    }

    if monthly_limit.is_none() || monthly_limit == Some(0.0) {
        let credit_bags = [
            raw.get("credits"),
            raw.get("creditBalance"),
            raw.get("usage"),
            cfg.get("credits"),
            cfg.get("includedCredits"),
            cfg.get("subscriptionCredits"),
            cfg.get("weeklyCredits"),
            cfg.get("sharedPool"),
        ];
        for bag in credit_bags.into_iter().flatten() {
            if let Some((t, u)) = credits_bag_usage(bag) {
                monthly_limit = Some(t);
                used = Some(u);
                break;
            }
        }
    }

    // prepaid 仅剩余：无 total 时作为「可用余额」展示
    if (monthly_limit.is_none() || monthly_limit == Some(0.0))
        && prepaid.is_some_and(|p| p > 0.0)
    {
        monthly_limit = prepaid;
        used = Some(0.0);
    }

    let remaining = match (monthly_limit, used) {
        (Some(limit), Some(u)) => Some((limit - u).max(0.0)),
        (Some(limit), None) => Some(limit),
        _ => None,
    };
    let usage_percent = match (monthly_limit, used) {
        (Some(limit), Some(u)) if limit > 0.0 => Some(((u / limit) * 100.0).clamp(0.0, 100.0)),
        _ => None,
    };
    let remaining_percent = usage_percent.map(|p| (100.0 - p).clamp(0.0, 100.0));

    let unlimited = (monthly_limit.is_none() || monthly_limit == Some(0.0))
        && (on_demand_cap.is_none() || on_demand_cap == Some(0.0))
        && prepaid.unwrap_or(0.0) <= 0.0
        && !cfg
            .get("isUnifiedBillingUser")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

    let mut exhausted = false;
    let mut exhaust_reason = None;
    if !unlimited {
        if let (Some(limit), Some(u)) = (monthly_limit, used) {
            if limit > 0.0 && u >= limit {
                exhausted = true;
                if on_demand_cap == Some(0.0) {
                    exhaust_reason = Some("按需/周额度已用尽（spending-limit）".into());
                } else {
                    exhaust_reason = Some(format!("额度已用尽（{:.0} / {:.0}）", u, limit));
                }
            }
        }
    }

    GrokQuota {
        monthly_limit,
        used,
        remaining,
        usage_percent,
        on_demand_cap,
        on_demand_used,
        prepaid_balance: prepaid,
        billing_period_start: period_str(
            cfg,
            &["billingPeriodStart", "billing_period_start"],
        ),
        billing_period_end: period_str(cfg, &["billingPeriodEnd", "billing_period_end"]),
        unlimited_or_free: Some(unlimited),
        exhausted: Some(exhausted),
        exhaust_reason,
        remaining_percent,
    }
}

async fn fetch_billing_and_user(token: &str) -> Result<(GrokQuota, Value, Value), String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(25))
        .build()
        .map_err(|e| format!("创建 HTTP 客户端失败: {}", e))?;
    let headers = cli_proxy_headers(token);

    let billing_url = format!("{}{}", CLI_CHAT_PROXY_BASE, BILLING_URL_PATH);
    let user_url = format!("{}{}", CLI_CHAT_PROXY_BASE, USER_URL_PATH);

    let billing_resp = client
        .get(&billing_url)
        .headers(headers.clone())
        .send()
        .await
        .map_err(|e| format!("请求 billing 失败: {}", e))?;
    let billing_status = billing_resp.status();
    let billing_text = billing_resp
        .text()
        .await
        .map_err(|e| format!("读取 billing 失败: {}", e))?;
    if !billing_status.is_success() {
        return Err(format!(
            "billing HTTP {}: {}",
            billing_status.as_u16(),
            billing_text.chars().take(200).collect::<String>()
        ));
    }
    let billing_raw: Value = serde_json::from_str(&billing_text)
        .map_err(|e| format!("解析 billing 失败: {}", e))?;
    let quota = normalize_billing(&billing_raw);

    let mut user_raw = Value::Null;
    if let Ok(user_resp) = client.get(&user_url).headers(headers).send().await {
        if user_resp.status().is_success() {
            if let Ok(text) = user_resp.text().await {
                if let Ok(v) = serde_json::from_str::<Value>(&text) {
                    user_raw = v;
                }
            }
        }
    }

    Ok((quota, billing_raw, user_raw))
}

/// 刷新 token + 实时额度
pub async fn refresh_account_token(account_id: &str) -> Result<GrokAccount, String> {
    let lock = token_refresh_lock(account_id);
    let _guard = lock.lock().await;

    let mut account =
        load_account(account_id).ok_or_else(|| format!("账号不存在: {}", account_id))?;

    // 1) 必要时刷新 access_token（refresh_token 单次有效，必须回写）
    if needs_token_refresh(&account) {
        let rt = account
            .refresh_token
            .clone()
            .ok_or_else(|| "账号缺少 refresh_token，请重新导入或登录".to_string())?;
        let client_id = account
            .oidc_client_id
            .clone()
            .unwrap_or_else(|| GROK_OIDC_CLIENT_ID.to_string());

        match refresh_access_token_http(&rt, &client_id).await {
            Ok((access, new_refresh, expires_in, scope)) => {
                account.access_token = access;
                if let Some(nr) = new_refresh {
                    account.refresh_token = Some(nr);
                }
                if let Some(sec) = expires_in {
                    let exp = now_ts() + sec;
                    account.expires_at = Some(exp);
                    account.expires_at_raw = Some(expires_at_to_iso(exp));
                } else if let Some(exp) = jwt_claim_i64(&account.access_token, "exp") {
                    account.expires_at = Some(exp);
                    account.expires_at_raw = Some(expires_at_to_iso(exp));
                }
                if let Some(s) = scope {
                    account.scope = Some(s);
                }
                account.token_updated_at = Some(now_ts());
                account.requires_reauth = Some(false);
                account.reauth_reason = None;
                account.status = Some("active".into());
                enrich_from_token(&mut account);
                // 同步 auth_raw，保证后续 build_auth_entry 覆盖的是新 token
                let (_, fresh_entry) = build_auth_entry(&account);
                account.auth_raw = Some(Value::Object(fresh_entry));

                // refresh_token 单次有效：当前注入账号必须同步写回 auth.json
                if resolve_current_account_id().as_deref() == Some(account_id) {
                    write_auth_json_for_account(&account).map_err(|e| {
                        format!(
                            "Token 已刷新但写回 auth.json 失败（CLI 可能仍用旧 refresh）: {}",
                            e
                        )
                    })?;
                }
            }
            Err(err) => {
                account.status = Some("error".into());
                account.status_reason = Some(err.clone());
                if err.contains("invalid")
                    || err.contains("revoked")
                    || err.contains("400")
                    || err.contains("401")
                {
                    account.requires_reauth = Some(true);
                    account.reauth_reason = Some(err.clone());
                }
                save_account(&account)?;
                return Err(err);
            }
        }
    }

    // 2) 实时额度
    match fetch_billing_and_user(&account.access_token).await {
        Ok((quota, billing_raw, user_raw)) => {
            account.quota = Some(quota);
            account.billing_raw = Some(billing_raw);
            if !user_raw.is_null() {
                if let Some(email) = user_raw.get("email").and_then(|v| v.as_str()) {
                    if !email.is_empty() {
                        account.email = email.to_string();
                    }
                }
                if let Some(fn_) = user_raw.get("firstName").and_then(|v| v.as_str()) {
                    account.first_name = Some(fn_.to_string());
                }
                if let Some(ln) = user_raw.get("lastName").and_then(|v| v.as_str()) {
                    account.last_name = Some(ln.to_string());
                }
                if let Some(uid) = user_raw.get("userId").and_then(|v| v.as_str()) {
                    account.user_id = Some(uid.to_string());
                }
                if let Some(flag) = user_raw.get("hasGrokCodeAccess").and_then(|v| v.as_bool()) {
                    account.has_grok_code_access = Some(flag);
                }
                account.user_raw = Some(user_raw);
            }
            account.usage_updated_at = Some(now_ts());
            account.subscription_query_last_success_at = Some(now_ts());
            account.quota_query_last_error = None;
            account.quota_query_last_error_at = None;
            account.status = Some("active".into());
            account.status_reason = None;
        }
        Err(err) => {
            account.quota_query_last_error = Some(err.clone());
            account.quota_query_last_error_at = Some(now_ts());
            // 额度失败不阻断 token 刷新结果
            logger::log_warn(&format!(
                "[Grok] 额度刷新失败: account_id={}, error={}",
                account_id, err
            ));
        }
    }

    enrich_from_token(&mut account);
    save_account(&account)?;
    Ok(account)
}

pub async fn refresh_all_tokens() -> Result<Vec<(String, Result<GrokAccount, String>)>, String> {
    let accounts = list_accounts();
    let mut results = Vec::new();
    for account in accounts {
        let id = account.id.clone();
        let result = refresh_account_token(&id).await;
        results.push((id, result));
    }
    Ok(results)
}

// ───────────────────────── Device Code OAuth ─────────────────────────

#[derive(Debug, Deserialize)]
struct DeviceCodeResponse {
    device_code: String,
    user_code: Option<String>,
    verification_uri: Option<String>,
    verification_uri_complete: Option<String>,
    expires_in: Option<u64>,
    interval: Option<u64>,
}

pub async fn oauth_login_start() -> Result<GrokOAuthStartResponse, String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(20))
        .build()
        .map_err(|e| format!("HTTP 客户端错误: {}", e))?;

    // discovery（容错使用常量）
    let mut token_endpoint = OIDC_TOKEN_URL.to_string();
    let mut device_endpoint = OIDC_DEVICE_URL.to_string();
    if let Ok(resp) = client.get(OIDC_DISCOVERY_URL).send().await {
        if resp.status().is_success() {
            if let Ok(v) = resp.json::<Value>().await {
                if let Some(t) = v.get("token_endpoint").and_then(|x| x.as_str()) {
                    token_endpoint = t.to_string();
                }
                if let Some(d) = v
                    .get("device_authorization_endpoint")
                    .and_then(|x| x.as_str())
                {
                    device_endpoint = d.to_string();
                }
            }
        }
    }

    let form = [
        ("client_id", GROK_OIDC_CLIENT_ID),
        ("scope", OIDC_SCOPES),
    ];
    let resp = client
        .post(&device_endpoint)
        .header(CONTENT_TYPE, "application/x-www-form-urlencoded")
        .header("Accept", "application/json")
        .form(&form)
        .send()
        .await
        .map_err(|e| format!("发起 device-code 失败: {}", e))?;
    let status = resp.status();
    let text = resp.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(format!(
            "device-code HTTP {}: {}",
            status.as_u16(),
            text.chars().take(200).collect::<String>()
        ));
    }
    let data: DeviceCodeResponse =
        serde_json::from_str(&text).map_err(|e| format!("解析 device-code 失败: {}", e))?;

    let login_id = uuid::Uuid::new_v4().to_string();
    // 官方默认 1800s；过短会导致用户登录还没完成就超时
    let expires_in = data.expires_in.unwrap_or(1800).max(60);
    let interval = data.interval.unwrap_or(5).max(1);
    let user_code = data.user_code.clone().unwrap_or_default();
    let device_complete = data
        .verification_uri_complete
        .clone()
        .or_else(|| {
            data.verification_uri.as_ref().map(|base| {
                if user_code.is_empty() {
                    base.clone()
                } else if base.contains('?') {
                    format!("{}&user_code={}", base, user_code)
                } else {
                    format!("{}?user_code={}", base, user_code)
                }
            })
        })
        .unwrap_or_else(|| {
            if user_code.is_empty() {
                "https://accounts.x.ai/oauth2/device".into()
            } else {
                format!("https://accounts.x.ai/oauth2/device?user_code={}", user_code)
            }
        });

    // 未登录时官方会跳到 sign-in + return_to=/oauth2/device?user_code=...
    // 直接打开该深链，减少一次跳转、避免用户落到 /account 页面迷路
    let verification_uri = if !user_code.is_empty() {
        let return_to = format!("/oauth2/device?user_code={}", user_code);
        format!(
            "https://accounts.x.ai/sign-in?redirect=oauth2-provider&return_to={}&email=true",
            urlencoding::encode(&return_to)
        )
    } else {
        device_complete.clone()
    };

    let session = GrokDeviceSession {
        device_code: data.device_code,
        verification_uri: verification_uri.clone(),
        user_code: if user_code.is_empty() {
            None
        } else {
            Some(user_code.clone())
        },
        interval_seconds: interval,
        expires_at: now_ts() + expires_in as i64,
        token_endpoint,
    };
    {
        let mut map = GROK_OAUTH_SESSIONS
            .lock()
            .map_err(|_| "OAuth session 锁异常".to_string())?;
        map.insert(login_id.clone(), session);
    }

    logger::log_info(&format!(
        "[Grok OAuth] device-code 已创建: user_code={}, expires_in={}, browser_url={}",
        if user_code.is_empty() { "<none>" } else { &user_code },
        expires_in,
        verification_uri
    ));

    Ok(GrokOAuthStartResponse {
        login_id,
        verification_uri,
        user_code: if user_code.is_empty() {
            None
        } else {
            Some(user_code)
        },
        expires_in,
        interval_seconds: interval,
        callback_url: Some(device_complete),
    })
}

pub async fn oauth_login_complete(login_id: &str) -> Result<GrokAccount, String> {
    let session = {
        let map = GROK_OAUTH_SESSIONS
            .lock()
            .map_err(|_| "OAuth session 锁异常".to_string())?;
        map.get(login_id)
            .cloned()
            .ok_or_else(|| "登录会话不存在或已取消".to_string())?
    };

    if now_ts() > session.expires_at {
        let _ = oauth_login_cancel(Some(login_id));
        return Err("登录已超时，请重试".into());
    }

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(20))
        .build()
        .map_err(|e| format!("HTTP 客户端错误: {}", e))?;

    let deadline = session.expires_at;
    let mut interval = session.interval_seconds;

    loop {
        if now_ts() > deadline {
            let _ = oauth_login_cancel(Some(login_id));
            return Err("登录已超时，请重试".into());
        }

        let form = [
            ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
            ("device_code", session.device_code.as_str()),
            ("client_id", GROK_OIDC_CLIENT_ID),
        ];
        let resp = client
            .post(&session.token_endpoint)
            .header(CONTENT_TYPE, "application/x-www-form-urlencoded")
            .header("Accept", "application/json")
            .form(&form)
            .send()
            .await
            .map_err(|e| format!("轮询 token 失败: {}", e))?;
        let status = resp.status().as_u16();
        let text = resp.text().await.unwrap_or_default();

        if status == 200 {
            let body: Value = serde_json::from_str(&text)
                .map_err(|e| format!("解析 token 响应失败: {}", e))?;
            let access = body
                .get("access_token")
                .and_then(|v| v.as_str())
                .ok_or_else(|| "token 响应缺少 access_token".to_string())?;
            let refresh = body
                .get("refresh_token")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            let expires_in = body.get("expires_in").and_then(|v| v.as_i64());
            let scope = body
                .get("scope")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());

            let mut entry = Map::new();
            entry.insert("key".into(), Value::String(access.to_string()));
            if let Some(rt) = refresh {
                entry.insert("refresh_token".into(), Value::String(rt));
            }
            entry.insert("auth_mode".into(), Value::String("oidc".into()));
            entry.insert(
                "oidc_client_id".into(),
                Value::String(GROK_OIDC_CLIENT_ID.into()),
            );
            entry.insert(
                "oidc_issuer".into(),
                Value::String(GROK_OIDC_ISSUER.into()),
            );
            if let Some(sec) = expires_in {
                let exp = now_ts() + sec;
                entry.insert("expires_at".into(), Value::String(expires_at_to_iso(exp)));
            }
            if let Some(s) = scope {
                entry.insert("scope".into(), Value::String(s));
            }

            // 补充 userinfo
            if let Ok(ui) = client
                .get(OIDC_USERINFO_URL)
                .bearer_auth(access)
                .send()
                .await
            {
                if ui.status().is_success() {
                    if let Ok(v) = ui.json::<Value>().await {
                        if let Some(email) = v.get("email").and_then(|x| x.as_str()) {
                            entry.insert("email".into(), Value::String(email.into()));
                        }
                        if let Some(sub) = v.get("sub").and_then(|x| x.as_str()) {
                            entry.insert("user_id".into(), Value::String(sub.into()));
                            entry.insert("principal_id".into(), Value::String(sub.into()));
                        }
                        if let Some(n) = v.get("given_name").and_then(|x| x.as_str()) {
                            entry.insert("first_name".into(), Value::String(n.into()));
                        }
                        if let Some(n) = v.get("family_name").and_then(|x| x.as_str()) {
                            entry.insert("last_name".into(), Value::String(n.into()));
                        }
                        if let Some(pic) = v.get("picture").and_then(|x| x.as_str()) {
                            entry.insert("profile_image_asset_id".into(), Value::String(pic.into()));
                        }
                    }
                }
            }

            let key = format!("{}{}", GROK_OIDC_SCOPE_PREFIX, GROK_OIDC_CLIENT_ID);
            // 按 user_id 合并已有账号，保留 tags/note/created_at
            let existing = entry
                .get("user_id")
                .and_then(|v| v.as_str())
                .or_else(|| entry.get("principal_id").and_then(|v| v.as_str()))
                .map(|uid| account_id_from_user(uid))
                .and_then(|id| load_account(&id))
                .or_else(|| {
                    let email = entry.get("email").and_then(|v| v.as_str()).unwrap_or("");
                    if email.is_empty() {
                        return None;
                    }
                    list_accounts()
                        .into_iter()
                        .find(|a| a.email.eq_ignore_ascii_case(email))
                });
            let mut account = account_from_auth_entry(&key, &entry, existing)?;
            save_account(&account)?;
            let _ = oauth_login_cancel(Some(login_id));

            // 登录后自动刷新额度并切到该号
            if let Ok(refreshed) = refresh_account_token(&account.id).await {
                account = refreshed;
            }
            inject_account(&account.id)?;
            account = load_account(&account.id).unwrap_or(account);
            return Ok(account);
        }

        // 每轮检查是否被取消
        {
            let map = GROK_OAUTH_SESSIONS
                .lock()
                .map_err(|_| "OAuth session 锁异常".to_string())?;
            if !map.contains_key(login_id) {
                return Err("登录已取消".into());
            }
        }

        // pending
        if text.contains("authorization_pending") || text.contains("slow_down") {
            if text.contains("slow_down") {
                interval = interval.saturating_add(5);
            }
            tokio::time::sleep(Duration::from_secs(interval)).await;
            continue;
        }
        if text.contains("expired_token") || text.contains("access_denied") {
            let _ = oauth_login_cancel(Some(login_id));
            return Err(format!("登录失败: {}", text.chars().take(200).collect::<String>()));
        }

        tokio::time::sleep(Duration::from_secs(interval)).await;
    }
}

pub fn oauth_login_cancel(login_id: Option<&str>) -> Result<(), String> {
    let mut map = GROK_OAUTH_SESSIONS
        .lock()
        .map_err(|_| "OAuth session 锁异常".to_string())?;
    if let Some(id) = login_id {
        map.remove(id);
    } else {
        map.clear();
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tier_mapping() {
        assert_eq!(map_tier_to_plan(Some(4)).1, "SUPERGROK");
        assert_eq!(map_tier_to_plan(Some(5)).1, "SUPERGROK_HEAVY");
        assert_eq!(map_tier_to_plan(Some(1)).1, "FREE");
    }

    #[test]
    fn billing_normalize() {
        let raw = json!({
            "config": {
                "monthlyLimit": {"val": 20000},
                "used": {"val": 3909},
                "onDemandCap": {"val": 0},
                "billingPeriodStart": "2026-07-01T00:00:00+00:00",
                "billingPeriodEnd": "2026-08-01T00:00:00+00:00"
            }
        });
        let q = normalize_billing(&raw);
        assert_eq!(q.monthly_limit, Some(20000.0));
        assert_eq!(q.used, Some(3909.0));
        assert!((q.remaining.unwrap() - 16091.0).abs() < 0.01);
        assert!(q.usage_percent.unwrap() > 19.0 && q.usage_percent.unwrap() < 20.0);
    }

    #[test]
    fn billing_normalize_format_credits_ondemand() {
        // format=credits 统一账期：常无 monthlyLimit，以 onDemandCap/used 为主
        let raw = json!({
            "config": {
                "currentPeriod": {
                    "type": "USAGE_PERIOD_TYPE_WEEKLY",
                    "start": "2026-07-07T00:00:00+00:00",
                    "end": "2026-07-14T00:00:00+00:00"
                },
                "onDemandCap": {"val": 100},
                "onDemandUsed": {"val": 25},
                "prepaidBalance": {"val": 0},
                "isUnifiedBillingUser": true
            }
        });
        let q = normalize_billing(&raw);
        assert_eq!(q.monthly_limit, Some(100.0));
        assert_eq!(q.used, Some(25.0));
        assert!((q.remaining.unwrap() - 75.0).abs() < 0.01);
        assert_eq!(
            q.billing_period_end.as_deref(),
            Some("2026-07-14T00:00:00+00:00")
        );
        assert_eq!(q.exhausted, Some(false));
    }

    #[test]
    fn billing_normalize_credits_exhausted_cap_zero() {
        let raw = json!({
            "config": {
                "onDemandCap": {"val": 0},
                "onDemandUsed": {"val": 0},
                "isUnifiedBillingUser": true
            }
        });
        let q = normalize_billing(&raw);
        assert_eq!(q.exhausted, Some(true));
        assert!(q.usage_percent.unwrap_or(0.0) >= 99.0);
    }

    #[test]
    fn auth_entry_roundtrip_key() {
        let account = GrokAccount {
            id: "grok_test".into(),
            email: "a@b.com".into(),
            name: None,
            first_name: Some("A".into()),
            last_name: Some("B".into()),
            user_id: Some("uid".into()),
            principal_id: Some("uid".into()),
            team_id: Some("team".into()),
            profile_image_asset_id: None,
            tier: Some(4),
            plan_type: Some("SUPERGROK".into()),
            plan_label: Some("SuperGrok".into()),
            access_token: "tok".into(),
            refresh_token: Some("rt".into()),
            scope: None,
            expires_at: Some(now_ts() + 3600),
            expires_at_raw: Some("2026-07-12T09:37:17.769Z".into()),
            oidc_issuer: Some(GROK_OIDC_ISSUER.into()),
            oidc_client_id: Some(GROK_OIDC_CLIENT_ID.into()),
            auth_entry_key: None,
            auth_mode_raw: Some("oidc".into()),
            create_time: Some("2026-07-12T03:34:46.853976Z".into()),
            coding_data_retention_opt_out: Some(false),
            has_grok_code_access: Some(true),
            quota: None,
            usage_updated_at: None,
            token_updated_at: None,
            status: None,
            status_reason: None,
            requires_reauth: None,
            reauth_reason: None,
            quota_query_last_error: None,
            quota_query_last_error_at: None,
            subscription_query_last_success_at: None,
            // 模拟官方未来可能多出的字段
            auth_raw: Some(json!({
                "key": "old_tok",
                "refresh_token": "old_rt",
                "future_field": "keep-me",
                "auth_mode": "oidc"
            })),
            userinfo_raw: None,
            billing_raw: None,
            user_raw: None,
            tags: None,
            account_note: None,
            created_at: now_ts(),
            last_used: now_ts(),
        };
        let (key, entry) = build_auth_entry(&account);
        assert_eq!(
            key,
            format!("{}{}", GROK_OIDC_SCOPE_PREFIX, GROK_OIDC_CLIENT_ID)
        );
        // 新 token 覆盖 auth_raw 旧值
        assert_eq!(entry.get("key").and_then(|v| v.as_str()), Some("tok"));
        assert_eq!(
            entry.get("refresh_token").and_then(|v| v.as_str()),
            Some("rt")
        );
        // 未知字段保留
        assert_eq!(
            entry.get("future_field").and_then(|v| v.as_str()),
            Some("keep-me")
        );
        // 官方 expires 原始串优先，避免改写成 .769000Z
        assert_eq!(
            entry.get("expires_at").and_then(|v| v.as_str()),
            Some("2026-07-12T09:37:17.769Z")
        );
        assert_eq!(entry.get("team_id").and_then(|v| v.as_str()), Some("team"));
        assert_eq!(
            entry.get("principal_type").and_then(|v| v.as_str()),
            Some("User")
        );
        assert!(!entry.contains_key("access_token"));
    }

    #[test]
    fn local_auth_json_field_set_parity() {
        // 与本机官方登录产物字段集对齐（不含密钥内容）
        let local_fields = [
            "auth_mode",
            "coding_data_retention_opt_out",
            "create_time",
            "email",
            "expires_at",
            "first_name",
            "key",
            "last_name",
            "oidc_client_id",
            "oidc_issuer",
            "principal_id",
            "principal_type",
            "refresh_token",
            "team_id",
            "user_id",
        ];
        let account = GrokAccount {
            id: "grok_uid".into(),
            email: "flor@example.com".into(),
            name: None,
            first_name: Some("Baker".into()),
            last_name: Some("David".into()),
            user_id: Some("7f619ed9-4bf2-45aa-ab6a-53e13b48f767".into()),
            principal_id: Some("7f619ed9-4bf2-45aa-ab6a-53e13b48f767".into()),
            team_id: Some("15612afe-df6e-49c6-bae9-c6698c9ae1cf".into()),
            profile_image_asset_id: None,
            tier: Some(4),
            plan_type: None,
            plan_label: None,
            access_token: "access".into(),
            refresh_token: Some("refresh".into()),
            scope: None,
            expires_at: Some(1),
            expires_at_raw: Some("2026-07-12T09:37:17.769Z".into()),
            oidc_issuer: Some(GROK_OIDC_ISSUER.into()),
            oidc_client_id: Some(GROK_OIDC_CLIENT_ID.into()),
            auth_entry_key: None,
            auth_mode_raw: Some("oidc".into()),
            create_time: Some("2026-07-12T03:34:46.853976Z".into()),
            coding_data_retention_opt_out: Some(false),
            has_grok_code_access: None,
            quota: None,
            usage_updated_at: None,
            token_updated_at: None,
            status: None,
            status_reason: None,
            requires_reauth: None,
            reauth_reason: None,
            quota_query_last_error: None,
            quota_query_last_error_at: None,
            subscription_query_last_success_at: None,
            auth_raw: None,
            userinfo_raw: None,
            billing_raw: None,
            user_raw: None,
            tags: None,
            account_note: None,
            created_at: now_ts(),
            last_used: now_ts(),
        };
        let (map_key, entry) = build_auth_entry(&account);
        assert_eq!(
            map_key,
            "https://auth.x.ai::b1a00492-073a-47ea-816f-4c329264a828"
        );
        for f in local_fields {
            assert!(entry.contains_key(f), "missing official field: {}", f);
        }
    }
}
