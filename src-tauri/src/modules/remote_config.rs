use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use super::config;
use super::logger;

const REMOTE_CONFIG_URL: &str =
    "https://raw.githubusercontent.com/jlcodes99/cockpit-tools/main/remote-config.json";
const REMOTE_CONFIG_CACHE_FILE: &str = "remote_config_cache.json";
const REMOTE_CONFIG_LOCAL_OVERRIDE_FILE: &str = "remote-config.local.json";
const CACHE_TTL_MS: i64 = 3_600_000;
const DEFAULT_REFRESH_INTERVAL_MS: i64 = 3_600_000;
const BUILTIN_HIDDEN_PLATFORM_IDS: &[&str] = &[];
const NEVER_REMOTE_HIDE_PLATFORM_IDS: &[&str] = &["claude_manager"];

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RemotePlatformOverride {
    #[serde(default)]
    pub hidden: bool,
    #[serde(default)]
    pub hidden_on: Vec<String>,
    #[serde(default = "default_target_versions")]
    pub target_versions: String,
    #[serde(default)]
    pub expires_at: Option<String>,
    #[serde(default)]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RemotePlatformRule {
    #[serde(default)]
    pub platform_ids: Vec<String>,
    #[serde(default)]
    pub hidden: bool,
    #[serde(default)]
    pub target_os: Vec<String>,
    #[serde(default = "default_target_versions")]
    pub target_versions: String,
    #[serde(default)]
    pub expires_at: Option<String>,
    #[serde(default)]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteConfigPayload {
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub refresh_interval_ms: Option<i64>,
    #[serde(default)]
    pub hidden_platform_ids: Vec<String>,
    #[serde(default)]
    pub platforms: BTreeMap<String, RemotePlatformOverride>,
    #[serde(default)]
    pub rules: Vec<RemotePlatformRule>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RemoteConfigCache {
    pub time: i64,
    pub data: RemoteConfigPayload,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteConfigAppliedRule {
    pub platform_ids: Vec<String>,
    #[serde(default)]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteConfigState {
    pub version: String,
    pub updated_at: i64,
    pub current_os: String,
    pub hidden_platform_ids: Vec<String>,
    pub applied_rules: Vec<RemoteConfigAppliedRule>,
    pub refresh_interval_ms: i64,
}

fn default_target_versions() -> String {
    "*".to_string()
}

fn empty_payload() -> RemoteConfigPayload {
    RemoteConfigPayload {
        version: String::new(),
        refresh_interval_ms: Some(DEFAULT_REFRESH_INTERVAL_MS),
        hidden_platform_ids: Vec::new(),
        platforms: BTreeMap::new(),
        rules: Vec::new(),
    }
}

fn get_shared_dir() -> Result<PathBuf, String> {
    let dir = config::get_shared_dir();
    if !dir.exists() {
        fs::create_dir_all(&dir).map_err(|e| format!("创建远端配置目录失败: {}", e))?;
    }
    Ok(dir)
}

fn get_cache_path() -> Result<PathBuf, String> {
    Ok(get_shared_dir()?.join(REMOTE_CONFIG_CACHE_FILE))
}

fn get_local_override_path() -> Result<PathBuf, String> {
    Ok(get_shared_dir()?.join(REMOTE_CONFIG_LOCAL_OVERRIDE_FILE))
}

fn get_workspace_remote_config_path() -> Option<PathBuf> {
    if !cfg!(debug_assertions) {
        return None;
    }
    Some(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("remote-config.json"),
    )
}

fn parse_remote_config_file(path: &Path) -> Result<RemoteConfigPayload, String> {
    let content = fs::read_to_string(path)
        .map_err(|e| format!("读取远端配置文件失败({}): {}", path.display(), e))?;
    serde_json::from_str::<RemoteConfigPayload>(&content)
        .map_err(|e| format!("解析远端配置文件失败({}): {}", path.display(), e))
}

fn load_local_remote_config() -> Result<Option<RemoteConfigPayload>, String> {
    if !cfg!(debug_assertions) {
        return Ok(None);
    }

    let local_override = get_local_override_path()?;
    if local_override.exists() {
        logger::log_info("[RemoteConfig] 使用本地覆盖文件 remote-config.local.json");
        return parse_remote_config_file(&local_override).map(Some);
    }

    if let Some(workspace_path) = get_workspace_remote_config_path() {
        if workspace_path.exists() {
            logger::log_info("[RemoteConfig] 使用工作区远端配置文件 remote-config.json");
            return parse_remote_config_file(&workspace_path).map(Some);
        }
    }

    Ok(None)
}

fn load_cache() -> Result<Option<RemoteConfigCache>, String> {
    let path = get_cache_path()?;
    if !path.exists() {
        return Ok(None);
    }
    let content = fs::read_to_string(&path).map_err(|e| format!("读取远端配置缓存失败: {}", e))?;
    if content.trim().is_empty() {
        return Ok(None);
    }
    if let Ok(cache) = serde_json::from_str::<RemoteConfigCache>(&content) {
        return Ok(Some(cache));
    }

    match crate::modules::atomic_write::quarantine_file(&path, "invalid-json") {
        Ok(Some(backup_path)) => logger::log_warn(&format!(
            "[RemoteConfig] 缓存解析失败，已隔离并忽略: path={}, backup={}",
            path.display(),
            backup_path.display()
        )),
        Ok(None) => logger::log_warn(&format!(
            "[RemoteConfig] 缓存解析失败，文件已不存在，忽略: path={}",
            path.display()
        )),
        Err(error) => logger::log_warn(&format!(
            "[RemoteConfig] 缓存解析失败，隔离失败，忽略: path={}, error={}",
            path.display(),
            error
        )),
    }
    Ok(None)
}

fn save_cache(payload: &RemoteConfigPayload) -> Result<(), String> {
    let cache = RemoteConfigCache {
        time: Utc::now().timestamp_millis(),
        data: payload.clone(),
    };
    let content = serde_json::to_string_pretty(&cache)
        .map_err(|e| format!("序列化远端配置缓存失败: {}", e))?;
    crate::modules::atomic_write::write_string_atomic(&get_cache_path()?, &content)
        .map_err(|e| format!("写入远端配置缓存失败: {}", e))
}

async fn fetch_remote_config() -> Result<RemoteConfigPayload, String> {
    logger::log_info("[RemoteConfig] 从远端拉取配置");

    let client = reqwest::Client::builder()
        .user_agent("Cockpit-Tools")
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|e| format!("创建远端配置 HTTP 客户端失败: {}", e))?;

    let url = format!("{}?t={}", REMOTE_CONFIG_URL, Utc::now().timestamp_millis());
    let response = client
        .get(url)
        .header("Cache-Control", "no-cache")
        .header("Pragma", "no-cache")
        .send()
        .await
        .map_err(|e| format!("拉取远端配置失败: {}", e))?;

    if !response.status().is_success() {
        return Err(format!("远端配置接口返回异常状态: {}", response.status()));
    }

    response
        .json()
        .await
        .map_err(|e| format!("解析远端配置失败: {}", e))
}

fn current_os() -> String {
    if cfg!(target_os = "windows") {
        "windows".to_string()
    } else if cfg!(target_os = "macos") {
        "macos".to_string()
    } else if cfg!(target_os = "linux") {
        "linux".to_string()
    } else {
        std::env::consts::OS.to_string()
    }
}

fn parse_version(value: &str) -> Vec<i64> {
    let trimmed = value.trim_start_matches(|c: char| !c.is_ascii_digit());
    trimmed
        .split(|c: char| !c.is_ascii_digit())
        .filter(|part| !part.is_empty())
        .map(|part| part.parse::<i64>().unwrap_or(0))
        .collect()
}

fn match_version(current_version: &str, pattern: &str) -> bool {
    let pattern = pattern.trim();
    if pattern.is_empty() || pattern == "*" {
        return true;
    }

    let (operator, version_str) = if let Some(rest) = pattern.strip_prefix(">=") {
        (">=", rest)
    } else if let Some(rest) = pattern.strip_prefix("<=") {
        ("<=", rest)
    } else if let Some(rest) = pattern.strip_prefix('>') {
        (">", rest)
    } else if let Some(rest) = pattern.strip_prefix('<') {
        ("<", rest)
    } else if let Some(rest) = pattern.strip_prefix('=') {
        ("=", rest)
    } else {
        ("=", pattern)
    };

    let current = parse_version(current_version);
    let target = parse_version(version_str);

    let mut cmp = 0;
    for idx in 0..3 {
        let c = *current.get(idx).unwrap_or(&0);
        let t = *target.get(idx).unwrap_or(&0);
        if c != t {
            cmp = if c > t { 1 } else { -1 };
            break;
        }
    }

    match operator {
        ">=" => cmp >= 0,
        "<=" => cmp <= 0,
        ">" => cmp > 0,
        "<" => cmp < 0,
        _ => cmp == 0,
    }
}

fn is_os_match(current_os: &str, target_os: &[String]) -> bool {
    if target_os.is_empty() || target_os.iter().any(|item| item.trim() == "*") {
        return true;
    }
    let current = current_os.to_ascii_lowercase();
    target_os
        .iter()
        .map(|item| item.trim().to_ascii_lowercase())
        .any(|item| item == current)
}

fn parse_datetime_millis(value: &str) -> Option<i64> {
    DateTime::parse_from_rfc3339(value)
        .ok()
        .map(|dt| dt.with_timezone(&Utc).timestamp_millis())
}

fn is_not_expired(expires_at: Option<&str>) -> bool {
    let Some(expires_at) = expires_at else {
        return true;
    };
    parse_datetime_millis(expires_at)
        .map(|expires_ms| expires_ms >= Utc::now().timestamp_millis())
        .unwrap_or(true)
}

fn normalize_platform_id(value: &str) -> Option<String> {
    let normalized = value.trim().to_ascii_lowercase().replace('_', "-");
    match normalized.as_str() {
        "antigravity" => Some("antigravity".to_string()),
        "antigravity-ide" => Some("antigravity_ide".to_string()),
        "codex" => Some("codex".to_string()),
        "claude-manager" => Some("claude_manager".to_string()),
        "zed" => Some("zed".to_string()),
        "github-copilot" | "githubcopilot" => Some("github-copilot".to_string()),
        "windsurf" => Some("windsurf".to_string()),
        "kiro" => Some("kiro".to_string()),
        "cursor" => Some("cursor".to_string()),
        "gemini" | "gemini-cli" => Some("gemini".to_string()),
        "codebuddy" => Some("codebuddy".to_string()),
        "codebuddy-cn" => Some("codebuddy_cn".to_string()),
        "qoder" => Some("qoder".to_string()),
        "trae" => Some("trae".to_string()),
        "trae-solo" => Some("trae_solo".to_string()),
        "trae-cn" => Some("trae_cn".to_string()),
        "trae-solo-cn" => Some("trae_solo_cn".to_string()),
        "workbuddy" => Some("workbuddy".to_string()),
        _ => None,
    }
}

fn normalize_platform_ids(values: &[String]) -> Vec<String> {
    let mut seen = BTreeSet::new();
    let mut result = Vec::new();
    for value in values {
        let Some(platform_id) = normalize_platform_id(value) else {
            continue;
        };
        if seen.insert(platform_id.clone()) {
            result.push(platform_id);
        }
    }
    result
}

fn should_apply_platform_override(
    item: &RemotePlatformOverride,
    current_os: &str,
    current_version: &str,
) -> bool {
    is_not_expired(item.expires_at.as_deref())
        && match_version(current_version, &item.target_versions)
        && (item.hidden || is_os_match(current_os, &item.hidden_on))
}

fn build_state(payload: RemoteConfigPayload, updated_at: i64) -> RemoteConfigState {
    let current_os = current_os();
    let current_version = env!("CARGO_PKG_VERSION");
    let mut hidden = BTreeSet::new();
    let mut applied_rules = Vec::new();

    for platform_id in BUILTIN_HIDDEN_PLATFORM_IDS
        .iter()
        .filter_map(|platform_id| normalize_platform_id(platform_id))
    {
        if hidden.insert(platform_id.clone()) {
            applied_rules.push(RemoteConfigAppliedRule {
                platform_ids: vec![platform_id],
                reason: Some(
                    "Temporarily hidden while Claude platform is being verified.".to_string(),
                ),
            });
        }
    }

    for platform_id in normalize_platform_ids(&payload.hidden_platform_ids) {
        hidden.insert(platform_id.clone());
        applied_rules.push(RemoteConfigAppliedRule {
            platform_ids: vec![platform_id],
            reason: None,
        });
    }

    for (platform_id, override_item) in &payload.platforms {
        if should_apply_platform_override(override_item, &current_os, current_version) {
            let normalized_ids = normalize_platform_ids(&[platform_id.clone()]);
            for normalized_id in &normalized_ids {
                hidden.insert(normalized_id.clone());
            }
            if !normalized_ids.is_empty() {
                applied_rules.push(RemoteConfigAppliedRule {
                    platform_ids: normalized_ids,
                    reason: override_item.reason.clone(),
                });
            }
        }
    }

    for rule in &payload.rules {
        if !rule.hidden {
            continue;
        }
        if !is_not_expired(rule.expires_at.as_deref()) {
            continue;
        }
        if !match_version(current_version, &rule.target_versions) {
            continue;
        }
        if !is_os_match(&current_os, &rule.target_os) {
            continue;
        }
        let normalized_ids = normalize_platform_ids(&rule.platform_ids);
        for platform_id in &normalized_ids {
            hidden.insert(platform_id.clone());
        }
        if !normalized_ids.is_empty() {
            applied_rules.push(RemoteConfigAppliedRule {
                platform_ids: normalized_ids,
                reason: rule.reason.clone(),
            });
        }
    }

    for platform_id in NEVER_REMOTE_HIDE_PLATFORM_IDS {
        hidden.remove(*platform_id);
    }
    applied_rules.iter_mut().for_each(|rule| {
        rule.platform_ids
            .retain(|platform_id| !NEVER_REMOTE_HIDE_PLATFORM_IDS.contains(&platform_id.as_str()));
    });
    applied_rules.retain(|rule| !rule.platform_ids.is_empty());

    let refresh_interval_ms = payload
        .refresh_interval_ms
        .filter(|value| *value >= 60_000)
        .unwrap_or(DEFAULT_REFRESH_INTERVAL_MS);

    RemoteConfigState {
        version: payload.version,
        updated_at,
        current_os,
        hidden_platform_ids: hidden.into_iter().collect(),
        applied_rules,
        refresh_interval_ms,
    }
}

async fn load_remote_config_raw(force_refresh: bool) -> Result<(RemoteConfigPayload, i64), String> {
    if let Some(local_data) = load_local_remote_config()? {
        return Ok((local_data, Utc::now().timestamp_millis()));
    }

    let cached = load_cache()?;
    let cache_is_fresh = cached
        .as_ref()
        .map(|cache| Utc::now().timestamp_millis() - cache.time < CACHE_TTL_MS)
        .unwrap_or(false);

    if !force_refresh {
        if let Some(cache) = cached.as_ref() {
            if cache_is_fresh {
                logger::log_info("[RemoteConfig] 使用本地缓存配置");
                return Ok((cache.data.clone(), cache.time));
            }
        }
    }

    match fetch_remote_config().await {
        Ok(payload) => {
            let updated_at = Utc::now().timestamp_millis();
            if let Err(err) = save_cache(&payload) {
                logger::log_warn(&format!("[RemoteConfig] 保存缓存失败: {}", err));
            }
            Ok((payload, updated_at))
        }
        Err(err) => {
            logger::log_warn(&format!(
                "[RemoteConfig] 拉取远端配置失败，尝试回退缓存: {}",
                err
            ));
            if let Some(cache) = cached {
                return Ok((cache.data, cache.time));
            }
            Ok((empty_payload(), Utc::now().timestamp_millis()))
        }
    }
}

pub async fn get_remote_config_state() -> Result<RemoteConfigState, String> {
    let (payload, updated_at) = load_remote_config_raw(false).await?;
    Ok(build_state(payload, updated_at))
}

pub async fn force_refresh_remote_config_state() -> Result<RemoteConfigState, String> {
    let (payload, updated_at) = load_remote_config_raw(true).await?;
    Ok(build_state(payload, updated_at))
}
