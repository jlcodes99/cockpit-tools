//! T3 Code 内部 Provider 实例（Codex + Antigravity + Claude）账号绑定与真实落盘切换。
//!
//! 严格对齐 T3 Code 上游 `origin/main` 落盘规范：
//! - `<stateDir>/settings.json` -> `providerInstances`
//! - Codex: `<stateDir>/providers/codex/<instanceId>/shadow` (OAuth overlay) 或 `cockpit-home` (完整 bundle / 本地网关)
//! - Antigravity: `<stateDir>/providers/antigravity/<sha256(instanceId)>/antigravity-acp/` (`acp_token.json` & `antigravity-oauth-token`)
//! - Claude (`claudeAgent`): `<stateDir>/providers/claudeAgent/<instanceId>` (`config.homePath` -> `CLAUDE_CONFIG_DIR`)

use chrono::{TimeZone, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock, Weak};
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;

use crate::models::t3code::{
    T3CodeBindableAccountView, T3CodeProviderInstanceView, T3CodeProviderRuntimeView,
    T3CodeProviderSwitchView, T3CodeProviderWorkspaceView, T3CodeQuotaRemaining, T3CodeQuotaWindowView,
};
use crate::modules::{
    account, atomic_write, claude_account, codex_account, codex_instance, codex_local_access,
    codex_provider_pool_quota, t3code_account, t3code_app_injection, t3code_instance,
};

#[path = "t3code_provider_live.rs"]
pub(super) mod live;
#[path = "t3code_provider_extensions.rs"]
pub(super) mod extensions;
#[path = "t3code_antigravity_identity.rs"]
mod antigravity_identity;
#[path = "t3code_provider_account_management.rs"]
mod management;
#[path = "t3code_provider_launch_paths.rs"]
pub(crate) mod launch_paths;

pub use management::{promote_provider_account, remove_provider_instance};
pub(crate) use management::resume_pending_defaults;

const BINDINGS_META_FILE: &str = "t3code_cockpit_bindings.json";
const ANTIGRAVITY_OAUTH_CLIENT_ID: &str =
    "1071006060591-tmhssin2h21lcre235vtolojh4g403ep.apps.googleusercontent.com";
const ANTIGRAVITY_OAUTH_CLIENT_SECRET: &str = "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";

pub const DRIVER_CODEX: &str = "codex";
pub const DRIVER_ANTIGRAVITY: &str = "antigravity";
pub const DRIVER_CLAUDE: &str = "claudeAgent";

const CODEX_SHARED_ENTRIES: &[&str] = &[
    "sessions",
    "archived_sessions",
    "sqlite",
    "shell_snapshots",
    "worktrees",
    "skills",
    "plugins",
    "cache",
    "logs",
];

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct PersistedProviderBinding {
    pub driver: String,
    pub account_id: String,
    pub updated_at: i64,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub unlinked: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verified_cache: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub live_generation: Option<String>,
}

pub(super) type PersistedBindingsMap = HashMap<String, PersistedProviderBinding>;

pub(super) fn reserve_provider_write(state_dir: &Path) -> Result<Arc<()>, String> {
    // Provider settings and binding metadata are shared by all providers in this T3 instance.
    static WRITES: OnceLock<Mutex<HashMap<PathBuf, Weak<()>>>> = OnceLock::new();
    let mut writes = WRITES
        .get_or_init(Default::default)
        .lock()
        .map_err(|error| error.to_string())?;
    writes.retain(|_, active| active.strong_count() > 0);
    if writes.get(state_dir).and_then(Weak::upgrade).is_some() {
        return Err(live::message("switchBusy"));
    }
    let reservation = Arc::new(());
    writes.insert(state_dir.to_path_buf(), Arc::downgrade(&reservation));
    Ok(reservation)
}

pub fn resolve_state_dir_for_t3_instance(
    t3_instance_id: Option<&str>,
) -> Result<(String, PathBuf), String> {
    let normalized = t3_instance_id
        .map(str::trim)
        .filter(|id| !id.is_empty() && *id != "__default__");
    let Some(instance_id) = normalized else {
        return Ok((
            "__default__".to_string(),
            t3code_account::default_state_dir()?,
        ));
    };
    let store = t3code_instance::load_instance_store()?;
    let profile = store
        .instances
        .into_iter()
        .find(|item| item.id == instance_id)
        .ok_or_else(|| format!("T3 Code 实例不存在: {}", instance_id))?;
    Ok((
        instance_id.to_string(),
        t3code_account::state_dir_for_instance_root(Path::new(&profile.user_data_dir)),
    ))
}

fn resolve_t3_running_for_instance(resolved_t3_id: &str) -> bool {
    if resolved_t3_id == "__default__" {
        return t3code_instance::load_instance_store()
            .ok()
            .and_then(|store| t3code_instance::resolve_pid(None, store.default_settings.last_pid))
            .is_some();
    }
    t3code_instance::load_instance_store()
        .ok()
        .and_then(|store| {
            store
                .instances
                .into_iter()
                .find(|item| item.id == resolved_t3_id)
        })
        .and_then(|profile| {
            t3code_instance::resolve_pid(Some(Path::new(&profile.user_data_dir)), profile.last_pid)
        })
        .is_some()
}

fn provider_cache_path(state_dir: &Path, instance_id: &str) -> PathBuf {
    let base_dir = state_dir.parent().unwrap_or(state_dir);
    base_dir.join("caches").join(format!("{instance_id}.json"))
}

fn remove_provider_status_cache(state_dir: &Path, instance_id: &str) {
    let cache_path = provider_cache_path(state_dir, instance_id);
    if cache_path.is_file() {
        let _ = fs::remove_file(cache_path);
    }
}

fn normalize_loopback_origin(raw: &str) -> Option<String> {
    let url = reqwest::Url::parse(raw.trim()).ok()?;
    let scheme = url.scheme();
    if scheme != "http" && scheme != "https" {
        return None;
    }
    let host = url.host_str()?.to_ascii_lowercase();
    let normalized_host = if host == "localhost" || host == "::1" || host == "[::1]" {
        "127.0.0.1".to_string()
    } else {
        host
    };
    let port = url.port_or_known_default()?;
    Some(format!("{scheme}://{normalized_host}:{port}"))
}

fn parse_provider_cache_value(
    value: &Value,
    instance_id: &str,
    driver: &str,
    config_obj: Option<&Map<String, Value>>,
    source: &str,
) -> Option<T3CodeProviderRuntimeView> {
    if value.get("driver").and_then(Value::as_str) != Some(driver) {
        return None;
    }
    let auth = value.get("auth");
    let auth_status = auth
        .and_then(|a| a.get("status"))
        .and_then(Value::as_str)
        .map(str::to_owned);
    let account_email = auth
        .and_then(|a| a.get("email"))
        .and_then(Value::as_str)
        .map(str::to_owned);
    let workspace_id = auth
        .and_then(|a| a.get("workspaceId"))
        .and_then(Value::as_str)
        .map(str::to_owned);
    let endpoint = auth.and_then(|a| a.get("endpoint")).and_then(Value::as_str);
    let status = value
        .get("status")
        .and_then(Value::as_str)
        .map(str::to_owned);
    let message = value
        .get("message")
        .and_then(Value::as_str)
        .map(str::to_owned);
    let checked_at = value
        .get("checkedAt")
        .and_then(Value::as_str)
        .map(str::to_owned);
    let retained = value
        .get("modelsRetained")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let model_count = if retained {
        0
    } else {
        value
            .get("models")
            .and_then(Value::as_array)
            .map(|models| models.len())
            .unwrap_or(0)
    };
    let availability = if source == "staged" {
        Some("staged".to_owned())
    } else {
        value
            .get("availability")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .or_else(|| (status.as_deref() == Some("ready")).then(|| "available".to_owned()))
    };

    let matches_gateway = if driver == DRIVER_CODEX {
        let dir = config_obj
            .and_then(|c| c.get("homePath").or_else(|| c.get("shadowHomePath")))
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(|p| {
                if let Some(rel) = p.strip_prefix("~/") {
                    dirs::home_dir()
                        .map(|h| h.join(rel))
                        .unwrap_or_else(|| PathBuf::from(p))
                } else {
                    PathBuf::from(p)
                }
            })
            .unwrap_or_else(codex_account::get_codex_home);
        let is_managed = fs::read_to_string(dir.join("config.toml"))
            .is_ok_and(|cfg| codex_local_access::is_cockpit_managed_local_access_config(&cfg));
        if is_managed {
            match (
                endpoint.and_then(normalize_loopback_origin),
                codex_account::configured_provider_base_url(&dir)
                    .as_deref()
                    .and_then(normalize_loopback_origin),
            ) {
                (Some(a), Some(b)) => a == b,
                _ => false,
            }
        } else {
            false
        }
    } else {
        false
    };

    Some(T3CodeProviderRuntimeView {
        instance_id: instance_id.to_owned(),
        installed: value.get("installed").and_then(Value::as_bool),
        account_email,
        workspace_id,
        auth_status,
        auth_uses_keyring: false,
        model_count,
        availability,
        matches_gateway,
        source: Some(source.to_owned()),
        status,
        message,
        checked_at,
    })
}

fn read_provider_cache_runtime(
    state_dir: &Path,
    instance_id: &str,
    driver: &str,
    config_obj: Option<&Map<String, Value>>,
) -> Option<T3CodeProviderRuntimeView> {
    let cache_path = provider_cache_path(state_dir, instance_id);
    let raw = fs::read_to_string(cache_path).ok()?;
    let value = serde_json::from_str::<Value>(&raw).ok()?;
    parse_provider_cache_value(&value, instance_id, driver, config_obj, "cache")
}

pub fn read_provider_runtime_status(t3_instance_id: &str) -> Vec<T3CodeProviderRuntimeView> {
    let observed = t3code_app_injection::observed_provider_status(t3_instance_id);
    if !observed.is_empty() {
        return observed;
    }
    let Ok((_, state_dir)) = resolve_state_dir_for_t3_instance(Some(t3_instance_id)) else {
        return Vec::new();
    };
    let Ok(settings) = read_t3_settings_json(&state_dir) else {
        return Vec::new();
    };
    settings["providerInstances"].as_object().into_iter().flat_map(|entries| entries.iter())
        .filter(|(id, entry)| entry["enabled"] != false && !id.contains(['/', '\\'])
            && !matches!(id.as_str(), "." | ".."))
        .filter_map(|(id, entry)| read_provider_cache_runtime(
            &state_dir, id, entry["driver"].as_str()?, entry["config"].as_object(),
        ))
        .collect()
}

fn is_valid_verified_antigravity_cache(cache: &Value, expected_email: Option<&str>) -> bool {
    let auth_ok = cache
        .pointer("/auth/status")
        .and_then(Value::as_str)
        .is_some_and(|s| s == "authenticated");
    let has_models = cache
        .get("models")
        .and_then(Value::as_array)
        .is_some_and(|m| !m.is_empty());
    let email_ok = match (
        expected_email.map(str::trim).filter(|s| !s.is_empty()),
        cache
            .pointer("/auth/email")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty()),
    ) {
        (Some(exp), Some(actual)) => exp.eq_ignore_ascii_case(actual),
        _ => true,
    };
    auth_ok && has_models && email_ok
}

fn antigravity_credentials_valid(state_dir: &Path, instance_id: &str) -> bool {
    let acp_dir = antigravity_profile_dir(state_dir, instance_id).join("antigravity-acp");
    let cli_ok = fs::read_to_string(acp_dir.join("antigravity-oauth-token"))
        .ok()
        .and_then(|raw| serde_json::from_str::<Value>(&raw).ok())
        .is_some_and(|val| {
            val.get("auth_method").and_then(Value::as_str) == Some("consumer")
                && val
                    .pointer("/token/refresh_token")
                    .and_then(Value::as_str)
                    .is_some_and(|t| !t.trim().is_empty())
        });
    let acp_ok = fs::read_to_string(acp_dir.join("acp_token.json"))
        .ok()
        .and_then(|raw| serde_json::from_str::<Value>(&raw).ok())
        .is_some_and(|val| {
            val.get("refresh_token")
                .and_then(Value::as_str)
                .is_some_and(|t| !t.trim().is_empty())
        });
    cli_ok && acp_ok
}

fn write_verified_antigravity_cache(
    state_dir: &Path,
    instance_id: &str,
    display_name: &str,
    enabled: bool,
    template: &Value,
) -> Result<(), String> {
    let cache_path = provider_cache_path(state_dir, instance_id);
    if let Some(parent) = cache_path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let mut payload = template.clone();
    if let Some(obj) = payload.as_object_mut() {
        obj.insert("displayName".to_string(), json!(display_name));
        obj.insert("enabled".to_string(), json!(enabled));
        obj.insert(
            "checkedAt".to_string(),
            json!(Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)),
        );
        let template_email = obj
            .get("auth")
            .and_then(|a| a.get("email"))
            .and_then(Value::as_str)
            .map(str::to_owned);
        if let Ok(prev_raw) = fs::read_to_string(&cache_path) {
            if let Ok(prev_val) = serde_json::from_str::<Value>(&prev_raw) {
                let prev_email = prev_val.pointer("/auth/email").and_then(Value::as_str);
                let same_account = match (template_email.as_deref(), prev_email) {
                    (Some(a), Some(b)) => a.eq_ignore_ascii_case(b),
                    (Some(_), None) => true,
                    _ => false,
                };
                if same_account {
                    if let Some(limits) = prev_val.get("usageLimits").cloned() {
                        obj.insert("usageLimits".to_string(), limits);
                    }
                }
            }
        }
    }
    let content = serde_json::to_string_pretty(&payload)
        .map_err(|e| format!("序列化 Antigravity 状态缓存失败: {}", e))?;
    atomic_write::write_string_atomic(&cache_path, &format!("{content}\n"))
        .map_err(|e| format!("写入 Antigravity 状态缓存失败: {}", e))
}

fn runtime_checked_before_binding(
    runtime: &T3CodeProviderRuntimeView,
    bound_updated_at: Option<i64>,
) -> bool {
    let Some(bound_ts) = bound_updated_at else {
        return false;
    };
    let Some(checked_str) = runtime.checked_at.as_deref() else {
        return false;
    };
    chrono::DateTime::parse_from_rfc3339(checked_str)
        .map(|dt| dt.timestamp() + 1 < bound_ts)
        .unwrap_or(false)
}

fn bindings_meta_path(state_dir: &Path) -> PathBuf {
    state_dir.join(BINDINGS_META_FILE)
}

fn load_persisted_bindings(state_dir: &Path) -> PersistedBindingsMap {
    let path = bindings_meta_path(state_dir);
    fs::read_to_string(path)
        .ok()
        .and_then(|content| serde_json::from_str(&content).ok())
        .unwrap_or_default()
}

fn save_persisted_bindings(
    state_dir: &Path,
    bindings: &PersistedBindingsMap,
) -> Result<(), String> {
    fs::create_dir_all(state_dir)
        .map_err(|error| format!("创建 T3 Code 状态目录失败: {}", error))?;
    let content = serde_json::to_string_pretty(bindings)
        .map_err(|error| format!("序列化 T3 Code 绑定记录失败: {}", error))?;
    atomic_write::write_string_atomic(&bindings_meta_path(state_dir), &content)
        .map_err(|error| format!("保存 T3 Code 绑定记录失败: {}", error))
}

pub(super) fn read_t3_settings_json(state_dir: &Path) -> Result<Value, String> {
    read_settings_object(&t3code_account::settings_path_for_state_dir(state_dir))
}

pub(super) fn read_settings_object(path: &Path) -> Result<Value, String> {
    let content = match fs::read_to_string(path) {
        Ok(content) => content,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(json!({})),
        Err(error) => return Err(format!("读取 T3 Code settings.json 失败: {error}")),
    };
    serde_json::from_str::<Value>(&content)
        .map_err(|error| format!("读取 T3 Code settings.json 失败: {error}"))
        .and_then(|value| {
            value
                .is_object()
                .then_some(value)
                .ok_or_else(|| "T3 Code settings.json 格式错误".to_string())
        })
}

pub(super) fn write_t3_settings_json(state_dir: &Path, value: &Value) -> Result<(), String> {
    fs::create_dir_all(state_dir)
        .map_err(|error| format!("创建 T3 Code 状态目录失败: {}", error))?;
    let path = t3code_account::settings_path_for_state_dir(state_dir);
    let content = serde_json::to_string_pretty(value)
        .map_err(|error| format!("序列化 T3 Code settings.json 失败: {}", error))?;
    atomic_write::write_string_atomic(&path, &content)
        .map_err(|error| format!("写入 T3 Code settings.json 失败: {}", error))
}

pub fn antigravity_instance_hash(instance_id: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(instance_id.as_bytes());
    format!("{:x}", hasher.finalize())
}

pub fn antigravity_profile_dir(state_dir: &Path, instance_id: &str) -> PathBuf {
    state_dir
        .join("providers")
        .join("antigravity")
        .join(antigravity_instance_hash(instance_id))
}

pub(super) fn format_unix_reset_iso(ts: Option<i64>) -> Option<String> {
    let ts = ts.filter(|value| *value > 0)?;
    Utc.timestamp_opt(ts, 0).single().map(|dt| dt.to_rfc3339())
}

fn codex_switch_badge_kind(account: &crate::models::codex::CodexAccount) -> Option<String> {
    if account.is_api_key_auth() {
        let provider_id = account.api_provider_id.as_deref().unwrap_or_default().trim().to_ascii_lowercase();
        if provider_id == "cockpit_chatgpt_plan" {
            return None;
        }
        let base_url = account.api_base_url.as_deref().map(str::trim).filter(|str| !str.is_empty());
        let official = match base_url {
            Some(value) => url::Url::parse(value).ok().is_some_and(|url| {
                url.scheme() == "https" && url.host_str() == Some("api.openai.com")
                    && url.port().is_none() && url.username().is_empty() && url.password().is_none()
                    && url.query().is_none() && url.fragment().is_none()
                    && matches!(url.path(), "/" | "/v1" | "/v1/")
            }),
            None => account.api_provider_mode == crate::models::codex::CodexApiProviderMode::OpenaiBuiltin,
        };
        let plan = account.plan_type.as_deref().unwrap_or_default().trim().to_ascii_lowercase();
        let kind = if official {
            "official-api"
        } else if matches!(provider_id.as_str(), "cockpit_api" | "new_api")
            || codex_account::is_cockpit_api_base_url(base_url)
            || matches!(plan.as_str(), "cockpit api" | "new_api_exclusive")
        {
            "cockpit-api"
        } else {
            "api-key"
        };
        return Some(kind.to_owned());
    }

    fn pro_tier(value: Option<&str>) -> Option<&'static str> {
        let normalized = value?.trim().to_ascii_lowercase().replace(['_', ' '], "-");
        match normalized.as_str() {
            "prolite" | "pro-lite" | "pro-5x" | "codex-pro-5x" | "pro-100" | "chatgptprolite" => Some("pro-100"),
            "pro" | "pro-20x" | "codex-pro-20x" | "pro-200" | "chatgptpro" => Some("pro-200"),
            "promax" | "pro-max" | "pro-500" | "chatgptpromax" => Some("pro-500"),
            _ => None,
        }
    }

    let plan = pro_tier(account.plan_type.as_deref())?;
    let original = account.quota.as_ref().and_then(|quota| quota.raw_data.as_ref())
        .and_then(|raw| raw.get("plan_type")).and_then(Value::as_str);
    let kind = if plan == "pro-200" {
        pro_tier(original).unwrap_or_else(|| {
            if pro_tier(account.auth_file_plan_type.as_deref()) == Some("pro-100") {
                "pro-100"
            } else {
                plan
            }
        })
    } else {
        plan
    };
    Some(kind.to_owned())
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum CodexSwitchEligibility {
    Eligible,
    Unverified,
    Unsupported,
}

fn is_t3_gpt_dialogue_model(model: &str) -> bool {
    let model = model.rsplit('/').next().unwrap_or_default().trim().to_ascii_lowercase();
    (model.starts_with("gpt-") || model.starts_with("codex-"))
        && !["image", "embed", "audio", "tts", "transcribe", "realtime", "moderation"]
            .iter().any(|kind| model.split('-').any(|part| part == *kind))
}

fn codex_switch_eligibility(account: &crate::models::codex::CodexAccount) -> CodexSwitchEligibility {
    use CodexSwitchEligibility::*;
    if account.is_agent_identity_auth() || account.is_web_session_auth()
        || codex_account::is_chatgpt_plan_account(account) || account.upstream_grok_account_id.is_some()
    {
        return Unsupported;
    }
    if !account.is_api_key_auth() || codex_switch_badge_kind(account).as_deref() == Some("official-api") {
        return Eligible;
    }
    let routes = &account.api_model_mappings;
    let has_gpt = routes.iter().any(|route| is_t3_gpt_dialogue_model(&route.upstream_model))
        || account.api_model_catalog.iter().any(|model| {
            let upstream = routes.iter().find(|route| route.client_model.eq_ignore_ascii_case(model))
                .map_or(model.as_str(), |route| route.upstream_model.as_str());
            is_t3_gpt_dialogue_model(upstream)
        });
    if has_gpt {
        Eligible
    } else if account.api_model_catalog.is_empty() && routes.is_empty() {
        Unverified
    } else {
        Unsupported
    }
}

fn build_codex_bindable_accounts(
    all_codex: Vec<crate::models::codex::CodexAccount>,
) -> (Vec<T3CodeBindableAccountView>, HashSet<String>, usize) {
    let api_service_id = codex_instance::CODEX_API_SERVICE_BIND_ACCOUNT_ID;
    let mut items = vec![T3CodeBindableAccountView {
        id: api_service_id.to_owned(),
        driver: DRIVER_CODEX.to_string(),
        email: "Cockpit Codex API Service".to_string(),
        display_name: Some("Codex API Service".to_string()),
        plan_badge: String::new(),
        plan_badge_kind: None,
        auth_mode: "api_service".to_string(),
        is_api_service: true,
        workspace_id: None,
        windows: Vec::new(),
        quota_summary_groups: None,
        provider_quota: None,
        primary_percent: None,
        secondary_percent: None,
        speed_tok_per_sec: None,
        cache_hit_percent: None,
    }];

    let mut switchable = HashSet::from([api_service_id.to_owned()]);
    let mut unverified = 0;
    // 2. 各 Codex 账号
    for acc in all_codex {
        if acc.is_agent_identity_auth() || acc.is_web_session_auth() {
            continue;
        }
        match codex_switch_eligibility(&acc) {
            CodexSwitchEligibility::Eligible => { switchable.insert(acc.id.clone()); }
            CodexSwitchEligibility::Unverified => unverified += 1,
            CodexSwitchEligibility::Unsupported => {}
        }
        let mut windows = Vec::new();
        let mut primary = None;
        let mut secondary = None;
        if let Some(quota) = &acc.quota {
            for (id, present, minutes, remaining, reset) in [
                (
                    "5h",
                    quota.hourly_window_present,
                    quota.hourly_window_minutes,
                    quota.hourly_percentage,
                    quota.hourly_reset_time,
                ),
                (
                    "weekly",
                    quota.weekly_window_present,
                    quota.weekly_window_minutes,
                    quota.weekly_percentage,
                    quota.weekly_reset_time,
                ),
            ] {
                if present == Some(false) || (acc.is_api_key_auth() && present != Some(true)) {
                    continue;
                }
                let remaining = remaining.clamp(0, 100);
                let label = match minutes.filter(|minutes| *minutes > 0) {
                    Some(minutes) if minutes % 1440 == 0 => format!("{}d", minutes / 1440),
                    Some(minutes) if minutes % 60 == 0 => format!("{}h", minutes / 60),
                    Some(minutes) => format!("{}m", minutes),
                    None => id.to_owned(),
                };
                if id == "5h" {
                    primary = Some(remaining);
                } else {
                    secondary = Some(remaining);
                }
                windows.push(T3CodeQuotaWindowView {
                    id: id.to_owned(),
                    label,
                    remaining: T3CodeQuotaRemaining::Percent { remaining_percent: remaining },
                    reset_time: format_unix_reset_iso(reset),
                });
            }
        }
        let plan_badge = acc
            .plan_type
            .clone()
            .or_else(|| acc.auth_file_plan_type.clone())
            .unwrap_or_else(|| {
                if acc.is_api_key_auth() {
                    "API_KEY".to_string()
                } else {
                    "UNKNOWN".to_string()
                }
            });
        items.push(T3CodeBindableAccountView {
            id: acc.id.clone(),
            driver: DRIVER_CODEX.to_string(),
            email: acc.email.clone(),
            display_name: acc
                .account_name
                .clone()
                .or_else(|| acc.api_provider_name.clone()),
            plan_badge,
            plan_badge_kind: codex_switch_badge_kind(&acc),
            auth_mode: if acc.is_api_key_auth() {
                "apikey".to_string()
            } else {
                "oauth".to_string()
            },
            is_api_service: false,
            workspace_id: acc.account_id.clone(),
            windows,
            quota_summary_groups: None,
            provider_quota: codex_provider_pool_quota::project(&acc, Utc::now().timestamp(), false),
            primary_percent: primary,
            secondary_percent: secondary,
            speed_tok_per_sec: None,
            cache_hit_percent: None,
        });
    }
    (items, switchable, unverified)
}

fn build_antigravity_bindable_accounts() -> Vec<T3CodeBindableAccountView> {
    let accounts = account::list_accounts().unwrap_or_default();
    let mut items = Vec::with_capacity(accounts.len());
    for acc in accounts {
        if acc.pending_oauth || acc.token.refresh_token.trim().is_empty() {
            continue;
        }
        let mut windows = Vec::new();
        let mut primary = None;
        let mut secondary = None;
        let mut plan_badge = "UNKNOWN".to_string();
        if let Some(quota) = &acc.quota {
            if let Some(tier) = quota
                .subscription_tier
                .as_deref()
                .or(quota.tier_id.as_deref())
                .map(str::trim)
                .filter(|v| !v.is_empty())
            {
                plan_badge = tier.to_string();
            }
            for (idx, model) in quota.models.iter().enumerate() {
                let pct = model.percentage.clamp(0, 100);
                if idx == 0 {
                    primary = Some(pct);
                } else if idx == 1 {
                    secondary = Some(pct);
                }
                windows.push(T3CodeQuotaWindowView {
                    id: model.name.clone(),
                    label: model
                        .display_name
                        .clone()
                        .unwrap_or_else(|| model.name.clone()),
                    remaining: T3CodeQuotaRemaining::Percent { remaining_percent: pct },
                    reset_time: (!model.reset_time.trim().is_empty())
                        .then(|| model.reset_time.clone()),
                });
            }
        }
        items.push(T3CodeBindableAccountView {
            id: acc.id.clone(),
            driver: DRIVER_ANTIGRAVITY.to_string(),
            email: acc.email.clone(),
            display_name: acc.name.clone(),
            plan_badge,
            plan_badge_kind: None,
            auth_mode: "oauth".to_string(),
            is_api_service: false,
            workspace_id: None,
            windows,
            quota_summary_groups: acc
                .quota
                .as_ref()
                .filter(|quota| !quota.quota_summary_stale)
                .and_then(|quota| quota.quota_summary_groups.clone()),
            provider_quota: None,
            primary_percent: primary,
            secondary_percent: secondary,
            speed_tok_per_sec: None,
            cache_hit_percent: None,
        });
    }
    items
}

fn build_claude_bindable_accounts() -> Vec<T3CodeBindableAccountView> {
    let accounts = claude_account::list_accounts();
    let mut items = Vec::with_capacity(accounts.len());
    for acc in accounts {
        if matches!(
            acc.auth_mode,
            crate::models::claude::ClaudeAuthMode::DesktopOAuth
                | crate::models::claude::ClaudeAuthMode::DesktopGateway
        ) {
            continue;
        }
        let mut windows = Vec::new();
        let mut primary = None;
        let mut secondary = None;
        if let Some(quota) = &acc.quota {
            let h = (100 - quota.five_hour_percentage.clamp(0, 100)).clamp(0, 100);
            let w = (100 - quota.seven_day_percentage.clamp(0, 100)).clamp(0, 100);
            primary = Some(h);
            secondary = Some(w);
            windows.push(T3CodeQuotaWindowView {
                id: "5h".to_string(),
                label: "5h".to_string(),
                remaining: T3CodeQuotaRemaining::Percent { remaining_percent: h },
                reset_time: format_unix_reset_iso(quota.five_hour_reset_time),
            });
            windows.push(T3CodeQuotaWindowView {
                id: "7d".to_string(),
                label: "7d".to_string(),
                remaining: T3CodeQuotaRemaining::Percent { remaining_percent: w },
                reset_time: format_unix_reset_iso(quota.seven_day_reset_time),
            });
            if let Some(used) = quota.seven_day_sonnet_percentage {
                windows.push(T3CodeQuotaWindowView {
                    id: "sonnet-weekly".to_string(),
                    label: "Sonnet · 7d".to_string(),
                    remaining: T3CodeQuotaRemaining::Percent { remaining_percent: 100 - used.clamp(0, 100) },
                    reset_time: format_unix_reset_iso(quota.seven_day_sonnet_reset_time),
                });
            }
        }
        let plan_badge = acc
            .plan_type
            .clone()
            .unwrap_or_else(|| match acc.auth_mode {
                crate::models::claude::ClaudeAuthMode::ApiKey => "API_KEY".to_string(),
                crate::models::claude::ClaudeAuthMode::DesktopGateway => "GATEWAY".to_string(),
                _ => "UNKNOWN".to_string(),
            });
        items.push(T3CodeBindableAccountView {
            id: acc.id.clone(),
            driver: DRIVER_CLAUDE.to_string(),
            email: acc.email.clone(),
            display_name: acc
                .organization_name
                .clone()
                .or_else(|| acc.api_provider_name.clone()),
            plan_badge,
            plan_badge_kind: None,
            auth_mode: format!("{:?}", acc.auth_mode).to_ascii_lowercase(),
            is_api_service: false,
            workspace_id: None,
            windows,
            quota_summary_groups: None,
            provider_quota: None,
            primary_percent: primary,
            secondary_percent: secondary,
            speed_tok_per_sec: None,
            cache_hit_percent: None,
        });
    }
    items
}

fn read_antigravity_binding_email(state_dir: &Path, instance_id: &str) -> Option<String> {
    let path = antigravity_profile_dir(state_dir, instance_id)
        .join("antigravity-acp")
        .join("antigravity-oauth-token");
    let raw = fs::read_to_string(path).ok()?;
    let value = serde_json::from_str::<Value>(&raw).ok()?;
    let email = value["id_token_email"].as_str()?.trim();
    (!email.is_empty()).then(|| email.to_owned())
}

fn unique_account(
    mut accounts: impl Iterator<Item = T3CodeBindableAccountView>,
) -> Option<T3CodeBindableAccountView> {
    let account = accounts.next()?;
    accounts.next().is_none().then_some(account)
}

fn detect_bound_account_from_disk(
    instance_id: &str,
    driver: &str,
    antigravity_email: Option<&str>,
    config_obj: Option<&Map<String, Value>>,
    persisted: &PersistedBindingsMap,
    candidates: &[T3CodeBindableAccountView],
) -> (Option<T3CodeBindableAccountView>, Option<String>) {
    let saved = persisted
        .get(instance_id)
        .filter(|saved| saved.driver == driver)
        .and_then(|saved| candidates.iter().find(|item| item.id == saved.account_id));

    let config_path = |key| {
        config_obj?
            .get(key)?
            .as_str()
            .map(str::trim)
            .filter(|path| !path.is_empty())
            .and_then(|path| {
                if let Some(relative) = path.strip_prefix("~/") {
                    return dirs::home_dir().map(|home| home.join(relative));
                }
                Some(PathBuf::from(path))
            })
    };

    match driver {
        DRIVER_ANTIGRAVITY => {
            let Some(email) = antigravity_email else {
                return (None, None);
            };
            let matches =
                |item: &&T3CodeBindableAccountView| item.email.eq_ignore_ascii_case(email);
            let account = saved
                .filter(matches)
                .cloned()
                .or_else(|| unique_account(candidates.iter().filter(matches).cloned()));
            (account, Some(email.to_owned()))
        }
        DRIVER_CODEX => {
            let dir = config_path("homePath")
                .or_else(|| config_path("shadowHomePath"))
                .unwrap_or_else(codex_account::get_codex_home);
            detect_codex_binding(&dir, saved, candidates)
        }
        DRIVER_CLAUDE => {
            let Some(dir) = config_path("homePath")
                .or_else(|| claude_account::get_default_claude_code_config_dir().ok())
            else {
                return (None, None);
            };
            detect_claude_binding(&dir, saved, candidates)
        }
        _ => (None, None),
    }
}

fn detect_codex_binding(
    dir: &Path,
    saved: Option<&T3CodeBindableAccountView>,
    candidates: &[T3CodeBindableAccountView],
) -> (Option<T3CodeBindableAccountView>, Option<String>) {
    let managed_service = fs::read_to_string(dir.join("config.toml"))
        .is_ok_and(|config| codex_local_access::is_cockpit_managed_local_access_config(&config));
    if managed_service {
        return (
            candidates.iter().find(|item| item.is_api_service).cloned(),
            None,
        );
    }
    let auth = fs::read_to_string(dir.join("auth.json"))
        .ok()
        .and_then(|raw| serde_json::from_str::<crate::models::codex::CodexAuthFile>(&raw).ok());
    let key_in_auth = auth
        .as_ref()
        .and_then(|auth| auth.openai_api_key.as_ref())
        .and_then(Value::as_str);
    if let Some(account) = saved
        .filter(|item| item.auth_mode == "apikey")
        .filter(|item| {
            codex_account::load_account(&item.id).is_some_and(|account| {
                if codex_local_access::profile_uses_provider_gateway_account(dir, &item.id) {
                    return true;
                }
                let Some(key) = account.openai_api_key.as_deref() else {
                    return false;
                };
                let configured = codex_account::configured_provider_base_url(dir);
                let normalize = |url: Option<&str>| {
                    url.unwrap_or("https://api.openai.com/v1")
                        .trim_end_matches('/')
                        .to_owned()
                };
                normalize(configured.as_deref()) == normalize(account.api_base_url.as_deref())
                    && (key_in_auth == Some(key)
                        || crate::modules::codex_app_injection::profile_uses_api_key(dir, key))
            })
        })
    {
        return (Some(account.clone()), None);
    }
    let Some(identity) = auth.and_then(codex_account::codex_auth_file_oauth_identity) else {
        return (None, None);
    };
    let matches = |item: &&T3CodeBindableAccountView| {
        item.auth_mode == "oauth"
            && item.email.eq_ignore_ascii_case(&identity.email)
            && match (&item.workspace_id, &identity.account_id) {
                (Some(expected), Some(observed)) => expected == observed,
                (None, None) => true,
                _ => false,
            }
    };
    let account = saved
        .filter(matches)
        .cloned()
        .or_else(|| unique_account(candidates.iter().filter(matches).cloned()));
    (account, Some(identity.email))
}

fn detect_claude_binding(
    dir: &Path,
    saved: Option<&T3CodeBindableAccountView>,
    candidates: &[T3CodeBindableAccountView],
) -> (Option<T3CodeBindableAccountView>, Option<String>) {
    let settings = fs::read_to_string(dir.join("settings.json"))
        .ok()
        .and_then(|raw| serde_json::from_str::<Value>(&raw).ok());
    if let Some(env) = settings.as_ref().and_then(|value| value.get("env")) {
        let auth_keys = [
            "ANTHROPIC_API_KEY",
            "ANTHROPIC_AUTH_TOKEN",
            "ANTHROPIC_BASE_URL",
        ];
        if auth_keys[..2].iter().any(|key| {
            env.get(key)
                .and_then(Value::as_str)
                .is_some_and(|value| !value.trim().is_empty())
        }) {
            let account = saved
                .filter(|item| item.auth_mode == "apikey")
                .filter(|item| {
                    claude_account::load_account(&item.id)
                        .and_then(|account| {
                            claude_account::build_api_key_cli_env_map(&account).ok()
                        })
                        .is_some_and(|expected| {
                            auth_keys.iter().all(|key| {
                                env.get(*key).and_then(Value::as_str)
                                    == expected.get(*key).map(String::as_str)
                            })
                        })
                });
            return (account.cloned(), None);
        }
    }
    let identity = claude_account::get_claude_code_global_config_path(dir)
        .ok()
        .and_then(|path| fs::read_to_string(path).ok())
        .and_then(|raw| serde_json::from_str::<Value>(&raw).ok());
    let Some(email) = identity
        .as_ref()
        .and_then(|value| value.pointer("/oauthAccount/emailAddress"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|email| !email.is_empty())
    else {
        return (None, None);
    };
    let matches = |item: &&T3CodeBindableAccountView| {
        item.auth_mode == "oauth" && item.email.eq_ignore_ascii_case(email)
    };
    let account = saved
        .filter(matches)
        .cloned()
        .or_else(|| unique_account(candidates.iter().filter(matches).cloned()));
    (account, Some(email.to_owned()))
}

pub(super) async fn refresh_bound_account_quota(
    driver: &str,
    account_id: &str,
) -> Result<(), String> {
    match driver {
        DRIVER_ANTIGRAVITY => {
            account::refresh_account_quota_in_background(account_id.to_owned()).await
        }
        DRIVER_CODEX => Box::pin(crate::modules::codex_quota::refresh_account_quota(
            account_id,
        ))
        .await
        .map(|_| ()),
        DRIVER_CLAUDE => {
            let account = Box::pin(claude_account::refresh_account_quota(account_id)).await?;
            match account.quota_error {
                Some(error) => Err(error.message),
                None => Ok(()),
            }
        }
        _ if extensions::is_extended_platform(driver) => {
            extensions::refresh_extended_account_quota(driver, account_id).await
        }
        _ => Err("Provider quota refresh unavailable".to_owned()),
    }
}

pub fn get_provider_workspace(
    t3_instance_id: Option<&str>,
) -> Result<T3CodeProviderWorkspaceView, String> {
    let (resolved_t3_id, state_dir) = resolve_state_dir_for_t3_instance(t3_instance_id)?;
    let settings = read_t3_settings_json(&state_dir)?;
    let runtime_status = t3code_app_injection::observed_provider_status(&resolved_t3_id);
    let t3_running = resolve_t3_running_for_instance(&resolved_t3_id);
    let t3_started_at = t3_running
        .then(|| fs::read(state_dir.join("server-runtime.json")).ok())
        .flatten()
        .and_then(|data| serde_json::from_slice::<Value>(&data).ok())
        .and_then(|state| state["startedAt"].as_str().map(str::to_owned));
    let injection_connected = t3code_app_injection::has_active_connection(&resolved_t3_id);
    let persisted = load_persisted_bindings(&state_dir);

    let (mut codex_accounts, codex_switchable, codex_unverified_count) =
        build_codex_bindable_accounts(codex_account::list_accounts());
    let antigravity_accounts = build_antigravity_bindable_accounts();
    let claude_accounts = build_claude_bindable_accounts();
    let extra_accounts = extensions::build_all_extra_bindable_accounts();

    let raw_instances = settings
        .get("providerInstances")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();

    let mut ordered_ids: Vec<(String, String, bool)> = vec![
        (DRIVER_CODEX.to_string(), DRIVER_CODEX.to_string(), true),
        (
            DRIVER_ANTIGRAVITY.to_string(),
            DRIVER_ANTIGRAVITY.to_string(),
            true,
        ),
        (DRIVER_CLAUDE.to_string(), DRIVER_CLAUDE.to_string(), true),
    ];

    for (inst_id, inst_val) in &raw_instances {
        let Some(raw_driver) = inst_val.get("driver").and_then(Value::as_str) else {
            continue;
        };
        let Some(logical_platform) = extensions::infer_platform_from_t3_entry(
            inst_id,
            raw_driver,
            inst_val.as_object(),
            &persisted,
        ) else {
            continue;
        };
        if !ordered_ids.iter().any(|(id, _, _)| id == inst_id) {
            let is_builtin = inst_id == extensions::PLATFORM_GROK && logical_platform == extensions::PLATFORM_GROK;
            ordered_ids.push((inst_id.clone(), logical_platform, is_builtin));
        }
    }

    let mut provider_instances = Vec::with_capacity(ordered_ids.len());
    let pending_bindings = live::bindings_during_switch(&state_dir);
    for (instance_id, driver, is_builtin_default) in ordered_ids {
        if let Some(previous) = pending_bindings.iter().find(|row| row.instance_id == instance_id) {
            provider_instances.push(previous.clone());
            continue;
        }
        let entry = raw_instances.get(&instance_id).and_then(Value::as_object);
        let config_obj = entry
            .and_then(|e| e.get("config"))
            .and_then(Value::as_object);
        let default_display = extensions::default_display_name(&driver);
        let raw_t3_driver = extensions::t3_raw_driver_for_platform(&driver)
            .unwrap_or(driver.as_str());
        let display_name = entry
            .and_then(|e| e.get("displayName"))
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or(if is_builtin_default {
                default_display
            } else {
                &instance_id
            })
            .to_string();
        let accent_color = entry
            .and_then(|e| e.get("accentColor"))
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(String::from);
        let enabled = entry
            .and_then(|e| e.get("enabled"))
            .and_then(Value::as_bool)
            .or_else(|| {
                config_obj
                    .and_then(|c| c.get("enabled"))
                    .and_then(Value::as_bool)
            })
            .unwrap_or(true);
        let home_path = config_obj
            .and_then(|c| c.get("homePath"))
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(String::from)
            .or_else(|| {
                if extensions::is_extended_platform(&driver) {
                    let dir = extensions::cockpit_provider_dir(&state_dir, &driver, &instance_id);
                    dir.exists().then(|| dir.to_string_lossy().to_string())
                } else {
                    None
                }
            });
        let shadow_home_path = config_obj
            .and_then(|c| c.get("shadowHomePath"))
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(String::from);
        let auth_method = config_obj
            .and_then(|c| c.get("authMethod"))
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(String::from);

        let antigravity_email = (driver == DRIVER_ANTIGRAVITY
            && auth_method
                .as_deref()
                .is_none_or(|mode| mode == "oauth-personal"))
        .then(|| read_antigravity_binding_email(&state_dir, &instance_id))
        .flatten();
        let (bound, observed_email) = if is_builtin_default && !enabled
            && persisted.get(&instance_id).is_some_and(|item| item.unlinked)
        {
            (None, None)
        } else if extensions::is_extended_platform(&driver) {
            let candidates = extra_accounts
                .get(&driver)
                .map(Vec::as_slice)
                .unwrap_or(&[]);
            extensions::detect_extended_bound_account(
                &state_dir,
                &instance_id,
                &driver,
                &persisted,
                candidates,
            )
        } else {
            detect_bound_account_from_disk(
                &instance_id,
                &driver,
                antigravity_email.as_deref(),
                config_obj,
                &persisted,
                match driver.as_str() {
                    DRIVER_CODEX => &codex_accounts,
                    DRIVER_ANTIGRAVITY => &antigravity_accounts,
                    DRIVER_CLAUDE => &claude_accounts,
                    _ => &[],
                },
            )
        };

        let bound_email_final = observed_email.or_else(|| bound.as_ref().map(|b| b.email.clone()));
        let persisted_entry = persisted
            .get(&instance_id)
            .filter(|item| {
                item.driver == driver && bound.as_ref().is_some_and(|account| account.id == item.account_id)
            });
        let bound_updated_at = persisted_entry.map(|item| item.updated_at);
        let binding_applied_live = persisted_entry
            .and_then(|item| item.live_generation.as_deref())
            .is_some_and(|generation| {
                raw_instances
                    .get(&instance_id)
                    .is_some_and(|entry| live::has_generation(entry, generation))
            });
        let persisted_verified_cache = persisted_entry
            .and_then(|item| item.verified_cache.as_ref())
            .filter(|cache| {
                is_valid_verified_antigravity_cache(cache, bound_email_final.as_deref())
            });

        let mut runtime = runtime_status
            .iter()
            .find(|item| item.instance_id == instance_id)
            .cloned()
            .or_else(|| {
                read_provider_cache_runtime(&state_dir, &instance_id, raw_t3_driver, config_obj)
            });

        let needs_repair = match driver.as_str() {
            DRIVER_ANTIGRAVITY => {
                let creds_valid = antigravity_credentials_valid(&state_dir, &instance_id);
                let observed_ready = runtime.as_ref().is_some_and(|rt| {
                    rt.availability.as_deref() == Some("available")
                        && rt.model_count > 0
                        && !runtime_checked_before_binding(rt, bound_updated_at)
                        && match (
                            rt.account_email
                                .as_deref()
                                .map(str::trim)
                                .filter(|s| !s.is_empty()),
                            bound_email_final
                                .as_deref()
                                .map(str::trim)
                                .filter(|s| !s.is_empty()),
                        ) {
                            (Some(a), Some(b)) => a.eq_ignore_ascii_case(b),
                            _ => true,
                        }
                });
                if !observed_ready && !runtime.as_ref().is_some_and(|runtime| runtime.installed == Some(false)) {
                    if let Some(verified) = persisted_verified_cache {
                        if !t3_running && creds_valid {
                            let _ = write_verified_antigravity_cache(
                                &state_dir,
                                &instance_id,
                                &display_name,
                                enabled,
                                verified,
                            );
                            runtime = parse_provider_cache_value(
                                verified,
                                &instance_id,
                                &driver,
                                config_obj,
                                "cache",
                            );
                        } else if t3_running {
                            runtime = parse_provider_cache_value(
                                verified,
                                &instance_id,
                                &driver,
                                config_obj,
                                "staged",
                            );
                        }
                    }
                }
                bound.is_some() && !creds_valid
            }
            DRIVER_CODEX => {
                if t3_running {
                    if let Some(rt) = runtime.as_mut() {
                        if runtime_checked_before_binding(rt, bound_updated_at) {
                            rt.source = Some("staged".to_owned());
                        }
                    }
                }
                bound.is_some() && shadow_home_path.is_some()
            }
            DRIVER_CLAUDE => {
                if t3_running {
                    if let Some(rt) = runtime.as_mut() {
                        if runtime_checked_before_binding(rt, bound_updated_at) {
                            rt.source = Some("staged".to_owned());
                        }
                    }
                }
                false
            }
            _ => {
                if t3_running {
                    if let Some(rt) = runtime.as_mut() {
                        if runtime_checked_before_binding(rt, bound_updated_at) {
                            rt.source = Some("staged".to_owned());
                        }
                    }
                }
                false
            }
        };

        if driver == DRIVER_ANTIGRAVITY && t3_running && bound_email_final.is_some() {
            if let Some(runtime) = runtime.as_mut() {
                let minimum = bound_updated_at.and_then(|at| u64::try_from(at).ok())
                    .and_then(|at| std::time::UNIX_EPOCH.checked_add(Duration::from_secs(at)));
                let profile = antigravity_profile_dir(&state_dir, &instance_id);
                if let Some(identity) = antigravity_identity::observed_identity(&profile, minimum, None) {
                    runtime.account_email = Some(identity.email);
                    runtime.auth_uses_keyring = identity.uses_keyring;
                } else if runtime.source.as_deref() != Some("live") {
                    runtime.account_email = None;
                }
            }
        }

        let model_refresh_pending = live::model_refresh_pending(&state_dir, &instance_id);
        let explicit_runtime_path = raw_instances.get(&instance_id)
            .is_some_and(|entry| launch_paths::t3_explicit_runtime_path(entry, &driver));
        provider_instances.push(T3CodeProviderInstanceView {
            instance_id,
            driver,
            display_name,
            accent_color,
            enabled,
            is_builtin_default,
            home_path,
            shadow_home_path,
            auth_method,
            bound_account_id: bound.as_ref().map(|b| b.id.clone()),
            bound_account_email: bound_email_final,
            bound_account_plan: bound.as_ref().map(|b| b.plan_badge.clone()),
            bound_account_plan_badge_kind: bound.as_ref().and_then(|b| b.plan_badge_kind.clone()),
            bound_is_api_service: bound.as_ref().is_some_and(|b| b.is_api_service),
            bound_account_workspace_id: bound.as_ref().and_then(|b| b.workspace_id.clone()),
            bound_updated_at,
            binding_applied_live,
            model_refresh_pending,
            pending_default: None,
            needs_repair,
            explicit_runtime_path,
            runtime,
            windows: bound
                .as_ref()
                .map(|b| b.windows.clone())
                .unwrap_or_default(),
            quota_summary_groups: bound.as_ref().and_then(|b| b.quota_summary_groups.clone()),
            provider_quota: bound.as_ref().and_then(|b| b.provider_quota.clone()),
            speed_tok_per_sec: bound.as_ref().and_then(|b| b.speed_tok_per_sec),
            cache_hit_percent: bound.as_ref().and_then(|b| b.cache_hit_percent),
        });
    }

    management::show_pending_defaults(&state_dir, &mut provider_instances);
    // Keep the full library for identifying existing bindings. Only offer selectable accounts.
    codex_accounts.retain(|account| codex_switchable.contains(&account.id));
    Ok(T3CodeProviderWorkspaceView {
        t3_instance_id: resolved_t3_id,
        state_dir: state_dir.to_string_lossy().to_string(),
        t3_running,
        t3_started_at,
        injection_connected,
        provider_instances,
        codex_accounts,
        codex_unverified_count,
        antigravity_accounts,
        claude_accounts,
        extra_accounts,
    })
}

fn remove_if_symlink(path: &Path) {
    if fs::symlink_metadata(path).is_ok_and(|meta| meta.file_type().is_symlink()) {
        let _ = fs::remove_file(path);
    }
}

/// 为 T3 Code 的 Codex Provider 实例准备完全隔离 config.toml 与 auth.json 的目录，
/// 仅共享 sessions/skills/plugins 等非凭据目录，防止默认 ~/.codex 的网关配置串号。
fn prepare_isolated_codex_home(
    shared_home: Option<&Path>,
    target_home: &Path,
) -> Result<(), String> {
    fs::create_dir_all(target_home)
        .map_err(|error| format!("创建 Codex 隔离目录失败: {}", error))?;
    remove_if_symlink(&target_home.join("config.toml"));
    remove_if_symlink(&target_home.join("auth.json"));
    remove_if_symlink(&target_home.join("auth.lock"));

    let Some(shared_home) = shared_home else {
        return Ok(());
    };
    if !shared_home.is_dir() {
        return Ok(());
    }
    for entry_name in CODEX_SHARED_ENTRIES {
        let target = shared_home.join(entry_name);
        if !target.exists() {
            let _ = fs::create_dir_all(&target);
        }
        let link = target_home.join(entry_name);
        if link.exists() || link.is_symlink() {
            continue;
        }
        #[cfg(unix)]
        {
            let _ = std::os::unix::fs::symlink(&target, &link);
        }
        #[cfg(windows)]
        {
            let _ = std::os::windows::fs::symlink_dir(&target, &link);
        }
    }
    Ok(())
}

#[cfg(unix)]
pub(super) fn restrict_dir(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let _ = fs::set_permissions(path, fs::Permissions::from_mode(0o700));
}

#[cfg(not(unix))]
pub(super) fn restrict_dir(_path: &Path) {}

#[cfg(unix)]
pub(super) fn restrict_file(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let _ = fs::set_permissions(path, fs::Permissions::from_mode(0o600));
}

#[cfg(not(unix))]
pub(super) fn restrict_file(_path: &Path) {}

pub async fn bind_provider_instance_account(
    t3_instance_id: Option<&str>,
    provider_instance_id: &str,
    driver: &str,
    cockpit_account_id: &str,
) -> Result<T3CodeProviderWorkspaceView, String> {
    let (_, state_dir) = resolve_state_dir_for_t3_instance(t3_instance_id)?;
    let _reservation = reserve_provider_write(&state_dir)?;
    check_account_available_async(t3_instance_id, provider_instance_id, driver, cockpit_account_id).await?;
    bind_provider_account(
        t3_instance_id,
        provider_instance_id,
        driver,
        cockpit_account_id,
        true,
    )
    .await
}

fn check_account_available(
    workspace: &T3CodeProviderWorkspaceView, target: &str, driver: &str, account_id: &str,
) -> Result<(), String> {
    let matches = |row: &T3CodeProviderInstanceView| {
        row.driver == driver && row.bound_account_id.as_deref() == Some(account_id)
    };
    if workspace.provider_instances.iter().any(|row| row.instance_id == target && matches(row)) {
        return Ok(());
    }
    if workspace.provider_instances.iter().any(matches) {
        return Err(live::message("accountAlreadyAdded"));
    }
    Ok(())
}

async fn check_account_available_async(
    instance: Option<&str>, target: &str, driver: &str, account_id: &str,
) -> Result<(), String> {
    let instance = instance.map(str::to_owned);
    let (target, driver, account_id) = (target.trim().to_owned(), driver.trim().to_owned(), account_id.trim().to_owned());
    tokio::task::spawn_blocking(move || {
        let workspace = get_provider_workspace(instance.as_deref())?;
        check_account_available(&workspace, &target, &driver, &account_id)
    }).await.map_err(|error| error.to_string())?
}

pub async fn switch_provider_instance_account(
    t3_instance_id: Option<&str>,
    provider_instance_id: &str,
    driver: &str,
    cockpit_account_id: &str,
) -> Result<T3CodeProviderSwitchView, String> {
    let id = t3_instance_id.map(str::to_owned);
    let (target_id, state_dir, workspace, reservation) = tokio::task::spawn_blocking(move || {
        let (target_id, state_dir) = resolve_state_dir_for_t3_instance(id.as_deref())?;
        let reservation = reserve_provider_write(&state_dir)?;
        let workspace = get_provider_workspace(Some(&target_id))?;
        Ok::<_, String>((target_id, state_dir, workspace, reservation))
    })
    .await
    .map_err(|error| error.to_string())??;
    check_account_available(&workspace, provider_instance_id, driver, cockpit_account_id)?;
    let provider = workspace
        .provider_instances
        .iter()
        .find(|row| row.instance_id == provider_instance_id && row.driver == driver)
        .ok_or_else(|| live::message("switchTargetChanged"))?;
    let accounts = match driver {
        DRIVER_CODEX => &workspace.codex_accounts,
        DRIVER_ANTIGRAVITY => &workspace.antigravity_accounts,
        DRIVER_CLAUDE => &workspace.claude_accounts,
        _ => workspace
            .extra_accounts
            .get(driver)
            .ok_or_else(|| live::message("switchTargetChanged"))?,
    };
    let account = accounts
        .iter()
        .find(|row| row.id == cockpit_account_id)
        .ok_or_else(|| live::message("switchTargetChanged"))?;
    let enabled = provider.enabled;
    let running = workspace.t3_running && enabled;
    if provider.bound_account_id.as_deref() == Some(cockpit_account_id)
        && provider.bound_updated_at.is_some()
        && !provider.needs_repair
        && (!running || provider.binding_applied_live)
    {
        return Ok(provider_switch_result(workspace, enabled, running));
    }
    let refresh_quota = !account.is_api_service
        && !(driver == DRIVER_CLAUDE && account.auth_mode == "apikey")
        && !(driver == extensions::PLATFORM_GROK && account.auth_mode == "apikey");
    let result = async {
        let mut runtime = if running {
            match live::ProviderControl::connect(&target_id, &state_dir).await {
                Ok(runtime) => runtime,
                Err(error) => {
                    crate::modules::logger::log_warn(&format!(
                        "[T3 Code] Saving account binding for next launch: {error}"
                    ));
                    None
                }
            }
        } else {
            None
        };
        if let Some(runtime) = runtime.as_mut() {
            if !extensions::is_extended_platform(driver) {
                let switched = runtime
                    .switch(&target_id, &state_dir, provider, account)
                    .await?;
                return Ok(provider_switch_result(switched, enabled, true));
            }
            runtime.prepare_switch(&state_dir, provider).await?;
        }
        let mut saved = bind_provider_account(
            Some(&target_id),
            provider_instance_id,
            driver,
            cockpit_account_id,
            false,
        )
        .await?;
        let mut applied_live = false;
        if let Some(mut runtime) = runtime {
            let (dir, id) = (state_dir.clone(), provider_instance_id.to_owned());
            let entry = tokio::task::spawn_blocking(move || {
                read_t3_settings_json(&dir)?["providerInstances"]
                    .get(&id)
                    .cloned()
                    .ok_or_else(|| live::message("switchTargetChanged"))
            })
            .await
            .map_err(|error| error.to_string())??;
            let reload = runtime.reload_entry_by_id(provider_instance_id, entry).await;
            if let Ok(generation) = reload {
                let (dir, id) = (state_dir.clone(), provider_instance_id.to_owned());
                tokio::task::spawn_blocking(move || {
                    let mut bindings = load_persisted_bindings(&dir);
                    let binding = bindings
                        .get_mut(&id)
                        .ok_or_else(|| live::message("switchTargetChanged"))?;
                    binding.live_generation = Some(generation);
                    save_persisted_bindings(&dir, &bindings)
                })
                .await
                .map_err(|error| error.to_string())??;
                applied_live = true;
                t3code_app_injection::forget_provider_status(&target_id, provider_instance_id);
                if let Some(row) = saved
                    .provider_instances
                    .iter_mut()
                    .find(|row| row.instance_id == provider_instance_id)
                {
                    row.binding_applied_live = true;
                    row.runtime = None;
                }
            } else if let Err(error) = reload {
                crate::modules::logger::log_warn(&format!(
                    "[T3 Code] Account binding saved; provider reload failed: {error}"
                ));
            }
        }
        Ok(provider_switch_result(saved, enabled, applied_live))
    }
    .await;

    drop(reservation);
    // A failed write can occur after some credential files have changed.
    t3code_app_injection::notify_provider_binding_changed(&target_id);
    if result.as_ref().is_ok_and(|switched| switched.applied_live) {
        if driver == DRIVER_ANTIGRAVITY {
            live::schedule_model_refresh(&target_id, &state_dir, provider_instance_id);
        }
        if refresh_quota {
            schedule_switched_account_quota(target_id, driver.to_owned(), account.id.clone());
        }
    }
    result
}

fn provider_switch_status(
    t3_running: bool,
    provider_enabled: bool,
    applied_live: bool,
) -> (bool, &'static str) {
    let restart_required = !applied_live && provider_enabled && t3_running;
    let key = match (applied_live, restart_required, provider_enabled) {
        (true, _, _) => "accounts.switchSuccess",
        (_, true, _) => "t3code.providerWorkspace.switchSaved",
        (_, _, true) => "t3code.providerWorkspace.readyOnLaunch",
        _ => "t3code.providerWorkspace.saved",
    };
    (restart_required, key)
}

fn provider_switch_result(
    workspace: T3CodeProviderWorkspaceView,
    provider_enabled: bool,
    applied_live: bool,
) -> T3CodeProviderSwitchView {
    let (restart_required, key) =
        provider_switch_status(workspace.t3_running, provider_enabled, applied_live);
    let message = crate::modules::i18n::translate(
        &crate::modules::config::get_user_config().language,
        key,
        &[],
    );
    T3CodeProviderSwitchView {
        workspace,
        applied_live,
        restart_required,
        message,
    }
}

fn schedule_switched_account_quota(t3_instance_id: String, driver: String, account_id: String) {
    tokio::spawn(async move {
        match tokio::time::timeout(
            Duration::from_secs(25),
            refresh_bound_account_quota(&driver, &account_id),
        )
        .await
        {
            Ok(Ok(())) => t3code_app_injection::notify_provider_binding_changed(&t3_instance_id),
            _ => crate::modules::logger::log_warn(
                "[T3 Code] Switched account quota refresh did not complete",
            ),
        }
    });
}

async fn bind_provider_account(
    t3_instance_id: Option<&str>,
    provider_instance_id: &str,
    driver: &str,
    cockpit_account_id: &str,
    prepare_and_verify: bool,
) -> Result<T3CodeProviderWorkspaceView, String> {
    let (target_id, state_dir) = resolve_state_dir_for_t3_instance(t3_instance_id)?;
    let instance_id = provider_instance_id.trim();
    let driver = driver.trim();
    let account_id = cockpit_account_id.trim();
    let settings = read_t3_settings_json(&state_dir)?;
    let raw_driver = extensions::t3_raw_driver_for_platform(driver)
        .ok_or_else(|| live::message("switchTargetChanged"))?;
    let mut entry = settings["providerInstances"]
        .get(instance_id)
        .cloned()
        .or_else(|| {
            (instance_id == driver || extensions::is_extended_platform(driver))
                .then(|| json!({ "driver": raw_driver, "enabled": true, "config": {} }))
        })
        .ok_or_else(|| live::message("switchTargetChanged"))?;
    let binding = prepare_provider_binding_entry(
        &state_dir, instance_id, driver, account_id, &mut entry, prepare_and_verify,
    ).await?;
    persist_provider_entry(&state_dir, instance_id, entry, Some(binding))?;
    tokio::task::spawn_blocking(move || get_provider_workspace(Some(&target_id)))
        .await
        .map_err(|error| error.to_string())?
}

async fn prepare_provider_binding_entry(
    state_dir: &Path,
    instance_id: &str,
    driver: &str,
    account_id: &str,
    entry: &mut Value,
    prepare_and_verify: bool,
) -> Result<PersistedProviderBinding, String> {
    if matches!(driver, DRIVER_ANTIGRAVITY | DRIVER_CLAUDE) {
        let state_dir = state_dir.to_owned();
        let driver = driver.to_owned();
        let mut current = entry.clone();
        *entry = tokio::task::spawn_blocking(move || {
            launch_paths::apply_native_path(&state_dir, &mut current, &driver);
            current
        }).await.map_err(|error| error.to_string())?;
    }
    let entry = entry.as_object_mut().ok_or_else(|| live::message("switchTargetChanged"))?;
    let raw_driver = extensions::t3_raw_driver_for_platform(driver)
        .ok_or_else(|| live::message("switchTargetChanged"))?;
    if instance_id.is_empty() || account_id.is_empty()
        || instance_id.contains(['/', '\\']) || matches!(instance_id, "." | "..")
        || entry.get("driver").and_then(Value::as_str) != Some(raw_driver)
    {
        return Err(live::message("switchTargetChanged"));
    }
    if entry.get("config").is_some_and(|config| !config.is_object()) {
        return Err(live::message("switchTargetChanged"));
    }
    let (routing, binding) = if extensions::is_extended_platform(driver) {
        let (dir, instance, platform, account) =
            (state_dir.to_path_buf(), instance_id.to_owned(), driver.to_owned(), account_id.to_owned());
        let outcome = tokio::task::spawn_blocking(move || {
            extensions::write_extended_account_binding(&dir, &instance, &platform, &account)
        })
        .await
        .map_err(|error| error.to_string())??;
        extensions::merge_managed_environment(entry, &outcome.environment, true);
        remove_provider_status_cache(state_dir, instance_id);
        (outcome.config_patch, outcome.binding)
    } else {
        write_provider_account(
            state_dir, instance_id, driver, account_id, Some(entry), prepare_and_verify,
        ).await?
    };
    let config = entry.entry("config").or_insert_with(|| json!({}))
        .as_object_mut().ok_or_else(|| live::message("switchTargetChanged"))?;
    apply_provider_routing(config, driver, &routing);
    antigravity_identity::isolate_personal_account(entry);
    Ok(binding)
}

fn persist_provider_entry(
    state_dir: &Path,
    instance_id: &str,
    entry: Value,
    binding: Option<PersistedProviderBinding>,
) -> Result<(), String> {
    let mut settings = read_t3_settings_json(state_dir)?;
    let instances = settings.as_object_mut().expect("validated settings")
        .entry("providerInstances").or_insert_with(|| json!({}))
        .as_object_mut().ok_or_else(|| live::message("switchTargetChanged"))?;
    instances.insert(instance_id.to_owned(), entry);
    let Some(binding) = binding else {
        return write_t3_settings_json(state_dir, &settings);
    };
    let mut bindings = load_persisted_bindings(state_dir);
    let previous = bindings.insert(instance_id.to_owned(), binding);
    save_persisted_bindings(state_dir, &bindings)?;
    if let Err(error) = write_t3_settings_json(state_dir, &settings) {
        if let Some(previous) = previous {
            bindings.insert(instance_id.to_owned(), previous);
        } else {
            bindings.remove(instance_id);
        }
        return match save_persisted_bindings(state_dir, &bindings) {
            Ok(()) => Err(error),
            Err(restore_error) => Err(format!("{error}\n{restore_error}")),
        };
    }
    Ok(())
}

fn apply_provider_routing(config: &mut Map<String, Value>, driver: &str, routing: &Map<String, Value>) {
    if driver == DRIVER_CODEX {
        config.remove("shadowHomePath");
    }
    config.extend(routing.clone());
}

fn set_provider_entry_enabled(entry: &mut Map<String, Value>, enabled: bool) {
    entry.insert("enabled".into(), json!(enabled));
    // T3 also treats the legacy config.enabled=false as disabled.
    if let Some(config) = entry.get_mut("config").and_then(Value::as_object_mut) {
        config.remove("enabled");
    }
}

fn inherited_codex_binary_path<'a>(
    settings: &'a Value,
    instance_id: &str,
    entry: Option<&Map<String, Value>>,
) -> Option<&'a str> {
    if instance_id == DRIVER_CODEX
        || entry
            .and_then(|entry| entry.get("config"))
            .and_then(|config| config["binaryPath"].as_str())
            .is_some_and(|path| !path.trim().is_empty())
    {
        return None;
    }
    settings["providerInstances"][DRIVER_CODEX]["config"]["binaryPath"]
        .as_str().map(str::trim).filter(|path| !path.is_empty())
}

fn save_provider_binding(
    state_dir: &Path,
    instance_id: &str,
    binding: PersistedProviderBinding,
) -> Result<(), String> {
    let mut bindings = load_persisted_bindings(state_dir);
    bindings.insert(instance_id.to_owned(), binding);
    save_persisted_bindings(state_dir, &bindings)
}

async fn write_provider_account(
    state_dir: &Path,
    instance_id: &str,
    driver: &str,
    account_id: &str,
    prepared_entry: Option<&Map<String, Value>>,
    prepare_and_verify: bool,
) -> Result<(Map<String, Value>, PersistedProviderBinding), String> {
    let instance_id = instance_id.to_owned();
    let driver = driver.to_owned();
    let account_id = account_id.to_owned();
    if instance_id.is_empty() || account_id.is_empty() {
        return Err("Provider 实例 ID 与绑定账号 ID 不能为空".to_string());
    }

    if instance_id.contains(['/', '\\']) || matches!(instance_id.as_str(), "." | "..") {
        return Err("Provider 实例 ID 无效".to_string());
    }
    let settings = read_t3_settings_json(state_dir)?;
    let entry_obj = prepared_entry.or_else(|| {
        settings["providerInstances"].get(&instance_id).and_then(Value::as_object)
    });
    let configured_driver = entry_obj
        .and_then(|entry| entry.get("driver"))
        .and_then(Value::as_str)
        .or_else(|| match instance_id.as_str() {
            DRIVER_CODEX => Some(DRIVER_CODEX),
            DRIVER_ANTIGRAVITY => Some(DRIVER_ANTIGRAVITY),
            DRIVER_CLAUDE => Some(DRIVER_CLAUDE),
            _ => None,
        });
    if configured_driver != Some(driver.as_str()) {
        return Err("Provider 实例已变更，请重新选择".to_string());
    }
    let ag_binary_path = entry_obj
        .and_then(|e| e.get("config"))
        .and_then(|c| c.get("binaryPath"))
        .and_then(Value::as_str)
        .map(str::to_owned);
    let ag_display_name = entry_obj
        .and_then(|e| e.get("displayName"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("Antigravity")
        .to_string();
    let ag_enabled = entry_obj
        .and_then(|e| e.get("enabled"))
        .and_then(Value::as_bool)
        .unwrap_or(true);

    let mut verified_cache = None;
    let mut routing = Map::new();

    match driver.as_str() {
        DRIVER_CODEX => {
            let is_api_service = codex_instance::is_api_service_bind_account_id(&account_id);
            let same_binding = load_persisted_bindings(state_dir).get(&instance_id)
                .is_some_and(|binding| binding.driver == DRIVER_CODEX && binding.account_id == account_id);
            if !same_binding
                && !is_api_service
                && !codex_account::load_account(&account_id).is_some_and(|account| {
                    codex_switch_eligibility(&account) == CodexSwitchEligibility::Eligible
                })
            {
                return Err(live::message("switchCodexRequiresGpt"));
            }
            let instance_root = state_dir.join("providers").join("codex").join(&instance_id);
            let target_home = instance_root.join("cockpit-home");
            let shared_home = dirs::home_dir().map(|h| h.join(".codex"));
            prepare_isolated_codex_home(shared_home.as_deref(), &target_home)?;

            if is_api_service {
                if !codex_local_access::prepare_local_access_for_bound_profile_dir(&target_home)
                    .await?
                {
                    return Err(crate::modules::i18n::translate(
                        &crate::modules::config::get_user_config().language,
                        "t3code.providerWorkspace.gatewayDisabled",
                        &[],
                    ));
                }
            } else {
                if prepare_and_verify {
                    codex_instance::inject_account_to_profile(&target_home, &account_id).await?;
                } else {
                    let (home, id) = (target_home.clone(), account_id.clone());
                    tokio::task::spawn_blocking(move || {
                        codex_account::write_saved_account_bundle_to_dir(&home, &id)
                    })
                    .await
                    .map_err(|error| error.to_string())??;
                }
            }
            routing.insert("setupMode".into(), json!("existing"));
            routing.insert("homePath".into(), json!(target_home));
            if let Some(binary) = inherited_codex_binary_path(&settings, &instance_id, entry_obj) {
                routing.insert("binaryPath".into(), json!(binary));
            }
            remove_provider_status_cache(state_dir, &instance_id);
        }
        DRIVER_ANTIGRAVITY => {
            let state_dir_clone = state_dir.to_path_buf();
            let instance_id_clone = instance_id.clone();
            let prepared = if prepare_and_verify {
                Some(account::prepare_account_for_injection(&account_id).await?)
            } else {
                None
            };
            let id = account_id.clone();
            let ag_email = tokio::task::spawn_blocking(move || {
                let account = match prepared {
                    Some(account) => account,
                    None => account::load_account(&id)?,
                };
                write_antigravity_account_to_t3_profile(&state_dir_clone, &instance_id_clone, &account)?;
                Ok::<_, String>(account.email)
            })
            .await
            .map_err(|e| format!("写入 T3 Antigravity 凭据失败: {}", e))??;

            routing.insert("authMethod".into(), json!("oauth-personal"));
            if prepare_and_verify {
                verified_cache = verify_and_cache_antigravity_profile(
                    state_dir,
                    &instance_id,
                    &ag_display_name,
                    ag_enabled,
                    ag_binary_path.as_deref(),
                    &ag_email,
                )
                .await?;
            } else {
                remove_provider_status_cache(state_dir, &instance_id);
            }
        }
        DRIVER_CLAUDE => {
            let state_dir_clone = state_dir.to_path_buf();
            let instance_id_clone = instance_id.clone();
            let account_id_clone = account_id.clone();
            let target_dir = state_dir.join("providers").join("claudeAgent").join(&instance_id);
            routing.insert("homePath".into(), json!(target_dir));
            tokio::task::spawn_blocking(move || {
                let claude = claude_account::load_account(&account_id_clone)
                    .ok_or_else(|| "Claude 账号不存在".to_string())?;
                if matches!(
                    claude.auth_mode,
                    crate::models::claude::ClaudeAuthMode::DesktopOAuth
                        | crate::models::claude::ClaudeAuthMode::DesktopGateway
                ) {
                    return Err(crate::modules::i18n::translate(
                        &crate::modules::config::get_user_config().language,
                        "t3code.providerWorkspace.claudeCodeOnly",
                        &[],
                    ));
                }
                fs::create_dir_all(&target_dir)
                    .map_err(|e| format!("创建 T3 Claude 实例目录失败: {}", e))?;
                claude_account::inject_to_claude_config(&account_id_clone, Some(&target_dir))?;
                if claude.auth_mode == crate::models::claude::ClaudeAuthMode::ApiKey {
                    clear_claude_oauth_account_if_present(&target_dir);
                }
                remove_provider_status_cache(&state_dir_clone, &instance_id_clone);
                Ok::<(), String>(())
            })
            .await
            .map_err(|e| format!("写入 T3 Claude 凭据失败: {}", e))??;
        }
        _ => return Err(format!("暂不支持的 T3 Code Provider 驱动: {}", driver)),
    }

    Ok((
        routing,
        PersistedProviderBinding {
            driver,
            account_id,
            updated_at: Utc::now().timestamp(),
            unlinked: false,
            verified_cache,
            live_generation: None,
        },
    ))
}

fn write_antigravity_account_to_t3_profile(
    state_dir: &Path,
    instance_id: &str,
    ag_account: &crate::models::Account,
) -> Result<(), String> {
    if ag_account.pending_oauth || ag_account.token.refresh_token.trim().is_empty() {
        return Err("所选 Antigravity 账号尚未完成 OAuth 授权".to_string());
    }
    let profile_dir = antigravity_profile_dir(state_dir, instance_id);
    let acp_dir = profile_dir.join("antigravity-acp");
    let tmp_dir = acp_dir.join("tmp");
    let profile_settings_path = acp_dir.join("settings.json");
    let mut profile_settings = read_settings_object(&profile_settings_path)?;
    let auth = profile_settings
        .as_object_mut()
        .expect("settings object")
        .entry("auth")
        .or_insert_with(|| json!({}))
        .as_object_mut()
        .ok_or_else(|| "Antigravity settings.json auth 格式错误".to_string())?;
    auth.insert("type".to_string(), json!("oauth-personal"));
    for dir in [&profile_dir, &acp_dir, &tmp_dir] {
        fs::create_dir_all(dir).map_err(|e| format!("创建 T3 Antigravity 实例目录失败: {}", e))?;
        restrict_dir(dir);
    }

    let mut acp_token = json!({
        "client_id": ANTIGRAVITY_OAUTH_CLIENT_ID,
        "client_secret": ANTIGRAVITY_OAUTH_CLIENT_SECRET,
        "refresh_token": ag_account.token.refresh_token,
    });
    if let Some(project_id) = ag_account
        .token
        .project_id
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        acp_token["project_id"] = json!(project_id);
    }
    let acp_token_path = acp_dir.join("acp_token.json");
    let acp_token_str = serde_json::to_string_pretty(&acp_token)
        .map_err(|e| format!("序列化 acp_token.json 失败: {}", e))?;

    let expiry_iso = Utc
        .timestamp_opt(ag_account.token.expiry_timestamp, 0)
        .single()
        .ok_or_else(|| "Antigravity Token 到期时间无效".to_string())?
        .to_rfc3339();
    let cli_token = json!({
        "auth_method": "consumer",
        "id_token_email": ag_account.email,
        "token": {
            "access_token": ag_account.token.access_token,
            "refresh_token": ag_account.token.refresh_token,
            "expires_in": ag_account.token.expires_in,
            "expiry": expiry_iso,
            "token_type": "Bearer"
        }
    });
    let cli_token_path = acp_dir.join("antigravity-oauth-token");
    let cli_token_str = serde_json::to_string_pretty(&cli_token)
        .map_err(|e| format!("序列化 antigravity-oauth-token 失败: {}", e))?;
    let profile_settings_str = serde_json::to_string(&profile_settings)
        .map_err(|e| format!("序列化 Antigravity settings.json 失败: {}", e))?;

    let files = [
        (&acp_token_path, &acp_token_str),
        (&cli_token_path, &cli_token_str),
        (&profile_settings_path, &profile_settings_str),
    ];
    let originals = files
        .iter()
        .map(|(path, _)| match fs::read(path) {
            Ok(bytes) => Ok(Some(bytes)),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error.to_string()),
        })
        .collect::<Result<Vec<_>, String>>()?;
    let mut written = 0;
    let result = (|| {
        for (path, contents) in files {
            atomic_write::write_string_atomic(path, contents)?;
            restrict_file(path);
            written += 1;
        }
        Ok::<(), String>(())
    })();
    if let Err(mut error) = result {
        // The ACP and CLI read different files for the same login.
        for ((path, _), previous) in files.into_iter().zip(originals).take(written) {
            let restored = match previous {
                Some(bytes) => atomic_write::write_bytes_atomic(path, &bytes),
                None => fs::remove_file(path).map_err(|error| error.to_string()),
            };
            if let Err(restore_error) = restored {
                error.push_str(&format!("\n{restore_error}"));
            }
        }
        return Err(error);
    }
    Ok(())
}

fn clear_claude_oauth_account_if_present(target_dir: &Path) {
    if let Ok(path) = claude_account::get_claude_code_global_config_path(target_dir) {
        if let Ok(raw) = fs::read_to_string(&path) {
            if let Ok(mut value) = serde_json::from_str::<Value>(&raw) {
                if let Some(obj) = value.as_object_mut() {
                    if obj.remove("oauthAccount").is_some() {
                        if let Ok(serialized) = serde_json::to_string_pretty(&value) {
                            let _ = atomic_write::write_string_atomic(&path, &serialized);
                        }
                    }
                }
            }
        }
    }
}

fn expand_tilde_path(raw: &str) -> PathBuf {
    let trimmed = raw.trim();
    if let Some(rel) = trimmed.strip_prefix("~/") {
        if let Some(home) = dirs::home_dir() {
            return home.join(rel);
        }
    }
    PathBuf::from(trimmed)
}



fn build_thinking_option_descriptors(
    config_options: &[Value],
    include_current: bool,
) -> Vec<Value> {
    let Some(opt) = config_options.iter().find(|c| {
        c.get("category").and_then(Value::as_str) == Some("thought_level")
            || (c.get("category").and_then(Value::as_str) == Some("thinking")
                && c.get("type").and_then(Value::as_str) == Some("select"))
    }) else {
        return Vec::new();
    };
    if opt.get("type").and_then(Value::as_str) != Some("select") {
        return Vec::new();
    }
    let Some(opt_id) = opt
        .get("id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
    else {
        return Vec::new();
    };
    let mut raw_choices: Vec<&Value> = Vec::new();
    if let Some(arr) = opt.get("options").and_then(Value::as_array) {
        for item in arr {
            if item.get("value").is_some() {
                raw_choices.push(item);
            } else if let Some(nested) = item.get("options").and_then(Value::as_array) {
                raw_choices.extend(nested.iter());
            }
        }
    }
    let mut seen = std::collections::HashSet::new();
    let mut choices = Vec::new();
    for cand in raw_choices {
        let Some(raw_val) = cand.get("value").and_then(Value::as_str) else {
            continue;
        };
        let choice_id = if raw_val.is_empty() {
            "_t3/empty"
        } else {
            let trimmed = raw_val.trim();
            if trimmed.is_empty() || trimmed != raw_val {
                continue;
            }
            trimmed
        };
        if !seen.insert(choice_id.to_string()) {
            continue;
        }
        let label = cand
            .get("name")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or(choice_id);
        let mut choice = json!({
            "id": choice_id,
            "label": label,
        });
        if let Some(desc) = cand
            .get("description")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            choice["description"] = json!(desc);
        }
        choices.push(choice);
    }
    if choices.is_empty() {
        return Vec::new();
    }
    let label = opt
        .get("name")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or(opt_id);
    let mut desc = json!({
        "id": opt_id,
        "label": label,
        "type": "select",
        "options": choices,
        "role": "reasoning",
    });
    if let Some(d) = opt
        .get("description")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        desc["description"] = json!(d);
    }
    if include_current {
        if let Some(cur) = opt.get("currentValue").and_then(Value::as_str) {
            let mapped = if cur.is_empty() { "_t3/empty" } else { cur };
            if seen.contains(mapped) {
                desc["currentValue"] = json!(mapped);
            }
        }
    }
    vec![desc]
}

fn build_antigravity_models_from_session(session_result: &Value) -> Vec<Value> {
    let config_options = session_result
        .get("configOptions")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let model_opt = config_options.iter().find(|opt| {
        opt.get("id").and_then(Value::as_str) == Some("model")
            || opt.get("category").and_then(Value::as_str) == Some("model")
    });
    let current_value = model_opt
        .and_then(|opt| opt.get("currentValue"))
        .and_then(Value::as_str)
        .or_else(|| {
            session_result
                .pointer("/models/currentModelId")
                .and_then(Value::as_str)
        });

    let mut raw_entries: Vec<(String, String)> = Vec::new();
    if let Some(options) = model_opt
        .and_then(|opt| opt.get("options"))
        .and_then(Value::as_array)
    {
        for entry in options {
            if let Some(val) = entry.get("value").and_then(Value::as_str) {
                let name = entry
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or(val)
                    .to_string();
                raw_entries.push((val.to_string(), name));
            } else if let Some(nested) = entry.get("options").and_then(Value::as_array) {
                for item in nested {
                    if let Some(val) = item.get("value").and_then(Value::as_str) {
                        let name = item
                            .get("name")
                            .and_then(Value::as_str)
                            .unwrap_or(val)
                            .to_string();
                        raw_entries.push((val.to_string(), name));
                    }
                }
            }
        }
    } else if let Some(avail) = session_result
        .pointer("/models/availableModels")
        .and_then(Value::as_array)
    {
        for model in avail {
            if let Some(val) = model.get("modelId").and_then(Value::as_str) {
                let name = model
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or(val)
                    .to_string();
                raw_entries.push((val.to_string(), name));
            }
        }
    }

    let mut seen_slugs = std::collections::HashSet::new();
    let mut seen_names = std::collections::HashSet::new();
    let mut models = Vec::new();
    for (slug, raw_name) in raw_entries {
        let slug_trimmed = slug.trim();
        if slug_trimmed.is_empty() || !seen_slugs.insert(slug_trimmed.to_string()) {
            continue;
        }
        let base_name = if raw_name.trim().is_empty() {
            slug_trimmed.to_string()
        } else {
            raw_name.trim().to_string()
        };
        let display_name = if seen_names.insert(base_name.clone()) {
            base_name
        } else {
            format!("{} ({})", base_name, slug_trimmed)
        };
        let is_default = current_value == Some(slug_trimmed);
        let option_descriptors = build_thinking_option_descriptors(&config_options, is_default);
        let mut obj = json!({
            "slug": slug_trimmed,
            "name": display_name,
            "isCustom": false,
            "capabilities": { "optionDescriptors": option_descriptors },
        });
        if is_default {
            obj["isDefault"] = json!(true);
            obj["aliases"] = json!(["antigravity-default"]);
        }
        models.push(obj);
    }
    models
}

async fn send_jsonrpc_line(
    stdin: &mut tokio::process::ChildStdin,
    payload: &Value,
) -> Result<(), String> {
    let line = format!("{payload}\n");
    stdin
        .write_all(line.as_bytes())
        .await
        .map_err(|err| format!("写入 Antigravity 验证请求失败: {}", err))?;
    stdin
        .flush()
        .await
        .map_err(|err| format!("刷新 Antigravity 验证请求失败: {}", err))
}

async fn verify_and_cache_antigravity_profile(
    state_dir: &Path,
    instance_id: &str,
    display_name: &str,
    enabled: bool,
    configured_binary: Option<&str>,
    account_email: &str,
) -> Result<Option<Value>, String> {
    let Some(agy_bin) = launch_paths::resolve_antigravity_for_probe(state_dir, configured_binary) else {
        remove_provider_status_cache(state_dir, instance_id);
        return Ok(None);
    };

    let profile_dir = antigravity_profile_dir(state_dir, instance_id);
    let acp_dir = profile_dir.join("antigravity-acp");
    let temp_root = acp_dir.join("tmp");
    fs::create_dir_all(&temp_root).map_err(|e| format!("创建 Antigravity 临时目录失败: {}", e))?;
    let probe_cwd = std::env::temp_dir().join(format!("t3-ag-probe-{}", uuid::Uuid::new_v4()));
    let _ = fs::create_dir_all(&probe_cwd);

    let mut cmd = Command::new(&agy_bin);
    cmd.arg("acp")
        .arg("--gemini-dir").arg(&profile_dir)
        .args(["--app-data-dir", "antigravity-acp"])
        .current_dir(&probe_cwd)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true);
    for key in [
        "GEMINI_API_KEY",
        "GOOGLE_API_KEY",
        "GOOGLE_APPLICATION_CREDENTIALS",
        "GOOGLE_CLOUD_PROJECT",
        "GOOGLE_CLOUD_LOCATION",
        "GOOGLE_CLOUD_QUOTA_PROJECT",
        "GOOGLE_GENAI_USE_VERTEXAI",
        "GCLOUD_PROJECT",
        "CLOUDSDK_CORE_PROJECT",
        "AGY_ACP_CCPA_PROJECT",
        "AGY_ACP_ENABLE_OAUTH",
        "BROWSER",
        "NODE_OPTIONS",
    ] {
        cmd.env_remove(key);
    }
    cmd.env("GEMINI_HOME", &profile_dir)
        .env("SSH_CONNECTION", "127.0.0.1 0 127.0.0.1 0")
        .env(antigravity_identity::FORCE_PROFILE_TOKEN, "1")
        .env("AGY_ACP_FORCE_FILE_STORAGE", "1")
        .env("AGY_CLI_DISABLE_AUTO_UPDATE", "1")
        .env("ELECTRON_RUN_AS_NODE", "1")
        .env("PYTHONUNBUFFERED", "1")
        .env("TMPDIR", &temp_root);
    #[cfg(target_os = "windows")]
    {
        cmd.env("TEMP", &temp_root)
            .env("TMP", &temp_root)
            .creation_flags(0x08000000);
    }

    let mut child = match cmd.spawn() {
        Ok(child) => child,
        Err(err) => {
            let _ = fs::remove_dir_all(&probe_cwd);
            return Err(format!("启动 Antigravity CLI 验证失败: {}", err));
        }
    };
    let probe_pid = child.id();
    let mut stdin = match child.stdin.take() {
        Some(stdin) => stdin,
        None => {
            let _ = child.kill().await;
            let _ = fs::remove_dir_all(&probe_cwd);
            return Err("无法打开 Antigravity CLI 标准输入".to_string());
        }
    };
    let stdout = match child.stdout.take() {
        Some(stdout) => stdout,
        None => {
            let _ = child.kill().await;
            let _ = fs::remove_dir_all(&probe_cwd);
            return Err("无法打开 Antigravity CLI 标准输出".to_string());
        }
    };

    let init_req = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {
            "protocolVersion": 1,
            "clientCapabilities": {
                "fs": { "readTextFile": false, "writeTextFile": false },
                "terminal": false
            },
            "clientInfo": { "name": "t3-code", "title": "T3 Code", "version": "0.1.11" }
        }
    });
    let auth_req = json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "authenticate",
        "params": { "methodId": "oauth-personal" }
    });
    let session_req = json!({
        "jsonrpc": "2.0",
        "id": 3,
        "method": "session/new",
        "params": {
            "cwd": probe_cwd.to_string_lossy().to_string(),
            "mcpServers": []
        }
    });

    let probe_fut = async {
        send_jsonrpc_line(&mut stdin, &init_req).await?;

        let mut version: Option<String> = None;
        let mut auth_ok = false;
        let mut session_result: Option<Value> = None;
        let mut auth_error: Option<String> = None;
        let mut lines = BufReader::new(stdout).lines();

        while let Ok(Some(line)) = lines.next_line().await {
            let trimmed = line.trim();
            if !trimmed.starts_with('{') {
                continue;
            }
            let Ok(msg) = serde_json::from_str::<Value>(trimmed) else {
                continue;
            };
            let Some(id) = msg.get("id").and_then(Value::as_i64) else {
                continue;
            };
            match id {
                1 => {
                    if let Some(err) = msg.get("error") {
                        auth_error = Some(
                            err.get("message")
                                .and_then(Value::as_str)
                                .unwrap_or("initialize failed")
                                .to_string(),
                        );
                        break;
                    }
                    version = msg
                        .pointer("/result/agentInfo/version")
                        .and_then(Value::as_str)
                        .map(str::to_owned);
                    send_jsonrpc_line(&mut stdin, &auth_req).await?;
                }
                2 => {
                    if let Some(err) = msg.get("error") {
                        auth_error = Some(
                            err.get("message")
                                .and_then(Value::as_str)
                                .unwrap_or("authenticate failed")
                                .to_string(),
                        );
                        break;
                    }
                    auth_ok = true;
                    send_jsonrpc_line(&mut stdin, &session_req).await?;
                }
                3 => {
                    if let Some(err) = msg.get("error") {
                        auth_error = Some(
                            err.get("message")
                                .and_then(Value::as_str)
                                .unwrap_or("session/new failed")
                                .to_string(),
                        );
                        break;
                    }
                    session_result = msg.get("result").cloned();
                    break;
                }
                _ => {}
            }
        }

        Ok::<_, String>((version, auth_ok, session_result, auth_error))
    };

    let outcome = tokio::time::timeout(Duration::from_secs(35), probe_fut).await;
    drop(stdin);
    let _ = child.kill().await;
    let _ = child.wait().await;
    let _ = fs::remove_dir_all(&probe_cwd);

    let (version, auth_ok, session_result, auth_error) = match outcome {
        Ok(Ok(tuple)) => tuple,
        Ok(Err(err)) => {
            remove_provider_status_cache(state_dir, instance_id);
            return Err(err);
        }
        Err(_) => {
            remove_provider_status_cache(state_dir, instance_id);
            return Err(
                "验证 Antigravity 登录状态超时，请检查本机 agy CLI 或网络后重试".to_string(),
            );
        }
    };

    if let Some(err) = auth_error {
        remove_provider_status_cache(state_dir, instance_id);
        return Err(format!("Antigravity 凭据验证失败（agy）: {}", err));
    }
    if !auth_ok {
        remove_provider_status_cache(state_dir, instance_id);
        return Err("Antigravity 凭据验证未完成，请检查网络或重新授权".to_string());
    }

    let profile = profile_dir.clone();
    let observed = tokio::task::spawn_blocking(move || {
        antigravity_identity::observed_identity(&profile, None, probe_pid)
    }).await.map_err(|error| error.to_string())?;
    let Some(identity) = observed.filter(|identity| {
        !identity.uses_keyring && identity.email.eq_ignore_ascii_case(account_email.trim())
    }) else {
        remove_provider_status_cache(state_dir, instance_id);
        return Err(live::message("switchVerificationFailed"));
    };

    let session_val = session_result.unwrap_or_else(|| json!({}));
    let models = build_antigravity_models_from_session(&session_val);
    let has_auto = session_val
        .pointer("/modes/availableModes")
        .and_then(Value::as_array)
        .is_some_and(|modes| {
            modes
                .iter()
                .any(|m| m.get("id").and_then(Value::as_str) == Some("auto"))
        });
    let supported_modes = if has_auto {
        json!([
            "approval-required",
            "auto-accept-edits",
            "auto",
            "full-access"
        ])
    } else {
        json!(["approval-required", "auto-accept-edits", "full-access"])
    };
    let remembered_modes = json!({
        "authMethod": "oauth-personal",
        "modes": supported_modes,
    });
    if let Ok(modes_str) = serde_json::to_string(&remembered_modes) {
        let _ = atomic_write::write_string_atomic(
            &profile_dir.join("t3-runtime-modes.json"),
            &modes_str,
        );
    }

    let auth_obj = json!({
        "status": "authenticated",
        "type": "oauth-personal",
        "label": "Google account",
        "email": identity.email,
    });

    let cache_payload = json!({
        "displayName": display_name,
        "showInteractionModeToggle": false,
        "supportedRuntimeModes": supported_modes,
        "enabled": enabled,
        "installed": true,
        "version": version,
        "status": "ready",
        "auth": auth_obj,
        "setup": {
            "canAuthenticate": true,
            "canInstall": true
        },
        "checkedAt": Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
        "models": models,
        "slashCommands": [],
        "skills": [],
        "supportsConversationRollback": true,
        "supportsTextGeneration": true,
        "instanceId": instance_id,
        "driver": DRIVER_ANTIGRAVITY,
        "continuation": {
            "groupKey": format!("antigravity:instance:{}", instance_id)
        }
    });
    write_verified_antigravity_cache(
        state_dir,
        instance_id,
        display_name,
        enabled,
        &cache_payload,
    )?;
    Ok(Some(cache_payload))
}

/// 在关闭旧 T3 Code 进程之后、拉起新 T3 Code 进程之前调用：
/// 恢复 Antigravity 已验证模型缓存（防止旧进程退出前冲掉磁盘缓存），并自动修复任何旧格式凭据或遗留的 Codex shadow 目录。
pub async fn prepare_before_t3_launch(t3_instance_id: Option<&str>) {
    let Ok((resolved_t3_id, state_dir)) = resolve_state_dir_for_t3_instance(t3_instance_id) else {
        return;
    };
    if let Err(error) = management::apply_pending_defaults_before_launch(&resolved_t3_id, &state_dir).await {
        crate::modules::logger::log_warn(&format!("[T3 Code] Could not apply saved default accounts before launch: {error}"));
    }
    let isolation = tokio::task::spawn_blocking({
        let state_dir = state_dir.clone();
        move || {
            let _write = reserve_provider_write(&state_dir)?;
            let bindings = load_persisted_bindings(&state_dir);
            let mut settings = read_t3_settings_json(&state_dir)?;
            let mut changed = false;
            if let Some(instances) = settings.get_mut("providerInstances").and_then(Value::as_object_mut) {
                for (id, entry) in instances {
                    if bindings.get(id).is_some_and(|binding| binding.driver == DRIVER_ANTIGRAVITY && !binding.unlinked) {
                        changed |= entry.as_object_mut().is_some_and(antigravity_identity::isolate_personal_account);
                    }
                }
            }
            if changed {
                write_t3_settings_json(&state_dir, &settings)?;
            }
            Ok::<(), String>(())
        }
    }).await;
    if !matches!(isolation, Ok(Ok(()))) {
        crate::modules::logger::log_warn("[T3 Code] Could not update Antigravity profile isolation before launch");
    }
    let Ok(workspace) = tokio::task::spawn_blocking({
        let id = resolved_t3_id.clone();
        move || get_provider_workspace(Some(&id))
    })
    .await
    .ok()
    .transpose()
    .ok()
    .flatten()
    .ok_or(()) else {
        return;
    };

    let settings = read_t3_settings_json(&state_dir).unwrap_or_else(|_| json!({}));
    let persisted = load_persisted_bindings(&state_dir);

    for inst in workspace.provider_instances {
        if !inst.enabled {
            continue;
        }
        let Some(bound_id) = inst.bound_account_id.as_deref() else {
            continue;
        };
        if inst.driver == DRIVER_ANTIGRAVITY {
            if !inst.needs_repair {
                if let Some(verified) = persisted
                    .get(&inst.instance_id)
                    .filter(|p| p.driver == DRIVER_ANTIGRAVITY && p.account_id == bound_id)
                    .and_then(|p| p.verified_cache.as_ref())
                    .filter(|c| {
                        is_valid_verified_antigravity_cache(c, inst.bound_account_email.as_deref())
                    })
                {
                    let _ = write_verified_antigravity_cache(
                        &state_dir,
                        &inst.instance_id,
                        &inst.display_name,
                        inst.enabled,
                        verified,
                    );
                    continue;
                }
            }
            if inst.needs_repair {
                let _ = bind_provider_instance_account(
                    Some(&resolved_t3_id),
                    &inst.instance_id,
                    &inst.driver,
                    bound_id,
                )
                .await;
            }
        } else if inst.driver == DRIVER_CODEX && inst.needs_repair {
            let _ = bind_provider_instance_account(
                Some(&resolved_t3_id),
                &inst.instance_id,
                &inst.driver,
                bound_id,
            )
            .await;
        } else {
            let _ = &settings;
        }
    }
    launch_paths::schedule_sync(Some(&resolved_t3_id));
}

pub async fn upsert_provider_instance(
    t3_instance_id: Option<&str>,
    instance_id: &str,
    driver: &str,
    display_name: Option<&str>,
    accent_color: Option<&str>,
    enabled: bool,
    bind_account_id: Option<&str>,
) -> Result<T3CodeProviderWorkspaceView, String> {
    let instance_id = instance_id.trim();
    let driver = driver.trim();
    if instance_id.is_empty()
        || !instance_id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-'))
    {
        return Err("Provider 实例 ID 仅允许字母、数字、下划线和连字符".to_string());
    }
    let Some(raw_t3_driver) = extensions::t3_raw_driver_for_platform(driver) else {
        return Err(format!("不支持的 Provider 驱动: {}", driver));
    };
    let (target_id, state_dir) = resolve_state_dir_for_t3_instance(t3_instance_id)?;
    let reservation = reserve_provider_write(&state_dir)?;
    let account_id = bind_account_id.map(str::trim).filter(|id| !id.is_empty());
    if let Some(account_id) = account_id {
        check_account_available_async(t3_instance_id, instance_id, driver, account_id).await?;
    }
    let settings = read_t3_settings_json(&state_dir)?;
    let mut entry = settings["providerInstances"]
        .get(instance_id)
        .cloned()
        .unwrap_or_else(|| json!({ "driver": raw_t3_driver, "config": {} }));
    let object = entry.as_object_mut().ok_or_else(|| live::message("switchTargetChanged"))?;
    object.insert("driver".into(), json!(raw_t3_driver));
    set_provider_entry_enabled(object, enabled);
    if let Some(name) = display_name.map(str::trim).filter(|s| !s.is_empty()) {
        object.insert("displayName".into(), json!(name));
    }
    if let Some(color) = accent_color.map(str::trim).filter(|s| !s.is_empty()) {
        object.insert("accentColor".into(), json!(color));
    }
    let binding = if let Some(account_id) = account_id {
        Some(prepare_provider_binding_entry(
            &state_dir, instance_id, driver, account_id, &mut entry, false,
        ).await?)
    } else {
        None
    };
    // Publish the configured provider once, with its executable and credentials already in place.
    persist_provider_entry(&state_dir, instance_id, entry, binding)?;
    drop(reservation);
    if enabled && account_id.is_some() && resolve_t3_running_for_instance(&target_id) {
        live::schedule_model_refresh(&target_id, &state_dir, instance_id);
    }
    tokio::task::spawn_blocking(move || get_provider_workspace(Some(&target_id)))
        .await
        .map_err(|error| error.to_string())?
}

pub async fn refresh_provider_account_quota(
    t3_instance_id: Option<&str>,
    provider_instance_id: &str,
) -> Result<T3CodeProviderWorkspaceView, String> {
    let t3_owned = t3_instance_id.map(str::to_owned);
    let provider_owned = provider_instance_id.trim().to_string();
    let workspace = tokio::task::spawn_blocking({
        let t3_owned = t3_owned.clone();
        move || get_provider_workspace(t3_owned.as_deref())
    })
    .await
    .map_err(|e| e.to_string())??;

    let provider = workspace
        .provider_instances
        .into_iter()
        .find(|item| item.instance_id == provider_owned)
        .ok_or_else(|| "Provider 实例不存在".to_string())?;

    if provider.bound_is_api_service {
        let ok = crate::modules::codex_app_injection::refresh_t3code_gateway_quota().await;
        if !ok {
            return Err("刷新 Codex API 服务额度失败".to_string());
        }
    } else if let Some(account_id) = provider.bound_account_id {
        refresh_bound_account_quota(&provider.driver, &account_id).await?;
    } else {
        return Err("该 Provider 实例尚未绑定账号".to_string());
    }

    tokio::task::spawn_blocking(move || get_provider_workspace(t3_owned.as_deref()))
        .await
        .map_err(|e| e.to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parallel_codex_inherits_only_the_default_executable_when_unset() {
        let settings = json!({"providerInstances": {"codex": {"config": {
            "binaryPath": "/example/codex", "homePath": "/example/default-home"
        }}}});
        let entry = json!({"config": {"homePath": "/example/parallel-home"}});
        assert_eq!(inherited_codex_binary_path(&settings, "parallel", entry.as_object()), Some("/example/codex"));
        assert_eq!(inherited_codex_binary_path(&settings, "codex", entry.as_object()), None);
        let custom = json!({"config": {"binaryPath": "/example/custom-codex"}});
        assert_eq!(inherited_codex_binary_path(&settings, "parallel", custom.as_object()), None);
        assert_eq!(inherited_codex_binary_path(&json!({}), "parallel", entry.as_object()), None);
    }
    use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};

    #[test]
    fn codex_candidates_require_a_chat_model_at_the_actual_upstream() {
        use crate::models::codex::{CodexAccount, CodexApiModelMapping, CodexApiProviderMode};
        use CodexSwitchEligibility::*;
        let mut key = CodexAccount::new_api_key(
            "test".into(), "test@example.test".into(), "fixture".into(),
            CodexApiProviderMode::Custom, Some("https://provider.example/v1".into()),
            None, None, vec![],
        );
        assert!(codex_switch_eligibility(&key) == Unverified);
        key.api_model_catalog = vec!["gpt-image-2".into(), "deepseek-chat".into()];
        assert!(codex_switch_eligibility(&key) == Unsupported);
        key.api_model_catalog.push("gpt-6-luna".into());
        key.api_model_mappings = vec![CodexApiModelMapping {
            client_model: "gpt-6-luna".into(), upstream_model: "deepseek-chat".into(),
        }];
        assert!(codex_switch_eligibility(&key) == Unsupported);
        key.api_model_catalog.push("openai/gpt-6.2-sol".into());
        assert!(codex_switch_eligibility(&key) == Eligible);

        let mut non_gpt = key.clone();
        non_gpt.id = "non-gpt".into();
        non_gpt.api_model_catalog = vec!["deepseek-chat".into()];
        let (accounts, switchable, _) = build_codex_bindable_accounts(vec![key, non_gpt]);
        let api_service_id = codex_instance::CODEX_API_SERVICE_BIND_ACCOUNT_ID;
        assert!(accounts
            .iter()
            .any(|account| account.id == api_service_id && account.is_api_service));
        assert_eq!(
            switchable,
            HashSet::from([api_service_id.to_owned(), "test".to_owned()])
        );
    }

    #[test]
    fn switch_reports_live_restart_launch_and_disabled_results() {
        assert_eq!(
            provider_switch_status(true, true, true),
            (false, "accounts.switchSuccess")
        );
        assert_eq!(
            provider_switch_status(true, true, false),
            (true, "t3code.providerWorkspace.switchSaved")
        );
        assert_eq!(
            provider_switch_status(false, true, false),
            (false, "t3code.providerWorkspace.readyOnLaunch")
        );
        assert_eq!(
            provider_switch_status(true, false, false),
            (false, "t3code.providerWorkspace.saved")
        );
    }

    fn oauth_account(
        id: &str,
        driver: &str,
        email: &str,
        workspace: Option<&str>,
    ) -> T3CodeBindableAccountView {
        serde_json::from_value(json!({
            "id": id, "driver": driver, "email": email, "workspaceId": workspace,
            "authMode": "oauth", "planBadge": "PRO", "isApiService": false, "windows": [],
        }))
        .unwrap()
    }

    #[test]
    fn codex_binding_follows_the_workspace_in_auth_file() {
        let dir = tempfile::tempdir().unwrap();
        let email = "same@example.test";
        let id_token = format!(
            "e30.{}.test",
            URL_SAFE_NO_PAD.encode(json!({"email": email}).to_string())
        );
        fs::write(
            dir.path().join("auth.json"),
            json!({
                "auth_mode": "chatgpt",
                "tokens": { "id_token": id_token, "access_token": "test", "account_id": "personal" }
            })
            .to_string(),
        )
        .unwrap();
        let accounts = [
            oauth_account("old", DRIVER_CODEX, email, Some("work")),
            oauth_account("new", DRIVER_CODEX, email, Some("personal")),
        ];
        let (bound, observed) = detect_codex_binding(dir.path(), Some(&accounts[0]), &accounts);
        assert_eq!(bound.unwrap().id, "new");
        assert_eq!(observed.as_deref(), Some(email));
    }

    #[test]
    fn claude_binding_follows_native_sign_in_and_sign_out() {
        let dir = tempfile::tempdir().unwrap();
        let config = dir.path().join(".config.json");
        let accounts = [
            oauth_account("old", DRIVER_CLAUDE, "old@example.test", None),
            oauth_account("new", DRIVER_CLAUDE, "new@example.test", None),
        ];
        fs::write(
            &config,
            r#"{"oauthAccount":{"emailAddress":"new@example.test"}}"#,
        )
        .unwrap();
        let observed = detect_claude_binding(dir.path(), Some(&accounts[0]), &accounts);
        assert_eq!(observed.0.unwrap().id, "new");
        fs::write(&config, "{}").unwrap();
        let observed = detect_claude_binding(dir.path(), Some(&accounts[0]), &accounts);
        assert!(observed.0.is_none() && observed.1.is_none());
    }

    #[test]
    fn antigravity_instance_hash_matches_upstream_sha256_rule() {
        // sha256("antigravity") in hex
        assert_eq!(
            antigravity_instance_hash("antigravity"),
            "ac0a3dfd6dddb20962cecff6ee5fe65e19d3923be20e52c5ab52ff877f7e4c32"
        );
    }

    #[test]
    fn isolated_codex_home_removes_symlinked_config_and_does_not_link_config() {
        let dir = tempfile::tempdir().unwrap();
        let shared = dir.path().join("shared");
        let isolated = dir.path().join("cockpit-home");
        fs::create_dir_all(&shared).unwrap();
        fs::create_dir_all(&isolated).unwrap();
        fs::write(
            shared.join("config.toml"),
            "model_provider = \"codex_local_access\"",
        )
        .unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(shared.join("config.toml"), isolated.join("config.toml"))
            .unwrap();

        prepare_isolated_codex_home(Some(&shared), &isolated).unwrap();
        assert!(!isolated.join("config.toml").exists());
        assert!(!isolated.join("config.toml").is_symlink());
        assert!(isolated.join("sessions").exists());
    }

    #[test]
    fn normalize_loopback_origin_treats_localhost_and_127_0_0_1_as_equal() {
        assert_eq!(
            normalize_loopback_origin("http://localhost:53823/v1"),
            normalize_loopback_origin("http://127.0.0.1:53823/v1")
        );
        assert_ne!(
            normalize_loopback_origin("http://localhost:53823/v1"),
            normalize_loopback_origin("http://127.0.0.1:53824/v1")
        );
    }

    #[test]
    fn build_antigravity_models_from_session_maps_default_and_options() {
        let session = json!({
            "configOptions": [{
                "id": "model",
                "category": "model",
                "currentValue": "gemini-4-hydrogen",
                "options": [
                    { "value": "gemini-4-hydrogen", "name": "Gemini 4 Hydrogen" },
                    { "value": "claude-sonnet-5-5", "name": "Claude Sonnet 5.5" }
                ]
            }, {
                "id": "thought_level",
                "name": "Thinking",
                "category": "thought_level",
                "type": "select",
                "currentValue": "high",
                "options": [
                    { "value": "low", "name": "Low" },
                    { "value": "high", "name": "High" }
                ]
            }]
        });
        let models = build_antigravity_models_from_session(&session);
        assert_eq!(models.len(), 2);
        assert_eq!(models[0]["slug"], "gemini-4-hydrogen");
        assert_eq!(models[0]["isDefault"], true);
        assert_eq!(models[0]["aliases"], json!(["antigravity-default"]));
        assert_eq!(
            models[0]["capabilities"]["optionDescriptors"][0]["role"],
            "reasoning"
        );
        assert_eq!(
            models[0]["capabilities"]["optionDescriptors"][0]["currentValue"],
            "high"
        );
        assert!(models[1].get("isDefault").is_none());
        assert!(models[1]["capabilities"]["optionDescriptors"][0]
            .get("currentValue")
            .is_none());
    }

    #[test]
    fn antigravity_credentials_valid_rejects_legacy_oauth_personal_method() {
        let dir = tempfile::tempdir().unwrap();
        let acp_dir = antigravity_profile_dir(dir.path(), "antigravity").join("antigravity-acp");
        fs::create_dir_all(&acp_dir).unwrap();
        fs::write(
            acp_dir.join("acp_token.json"),
            r#"{"refresh_token":"rt-123"}"#,
        )
        .unwrap();
        fs::write(
            acp_dir.join("antigravity-oauth-token"),
            r#"{"auth_method":"oauth-personal","token":{"refresh_token":"rt-123"}}"#,
        )
        .unwrap();
        assert!(!antigravity_credentials_valid(dir.path(), "antigravity"));

        fs::write(
            acp_dir.join("antigravity-oauth-token"),
            r#"{"auth_method":"consumer","token":{"refresh_token":"rt-123"}}"#,
        )
        .unwrap();
        assert!(antigravity_credentials_valid(dir.path(), "antigravity"));
    }
}
