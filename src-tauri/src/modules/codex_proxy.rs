use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::fs;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::Duration;
use tauri::Manager;
use url::Url;

use crate::modules::{atomic_write, codex_account, config, logger};

const CODEX_PROXY_CONFIG_FILE: &str = "codex_proxy_config.json";
const LEGACY_CCX_GATEWAY_CONFIG_FILE: &str = "ccx_gateway_config.json";
const DEFAULT_GATEWAY_BASE_URL: &str = "http://127.0.0.1:53000";

#[cfg(target_os = "windows")]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexProxyGatewayConfig {
    pub gateway_base_url: String,
    pub proxy_access_key: String,
    pub admin_access_key: String,
    pub binary_path: String,
    pub auto_start: bool,
}

impl Default for CodexProxyGatewayConfig {
    fn default() -> Self {
        Self {
            gateway_base_url: DEFAULT_GATEWAY_BASE_URL.to_string(),
            proxy_access_key: String::new(),
            admin_access_key: String::new(),
            binary_path: String::new(),
            auto_start: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexProxyGatewayHealth {
    pub running: bool,
    pub gateway_base_url: String,
    pub status: Option<u16>,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexProxyModelMapping {
    pub source: String,
    pub target: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexProxyResponsesChannelInput {
    pub gateway_base_url: String,
    pub proxy_access_key: String,
    pub admin_access_key: String,
    #[serde(default)]
    pub channel_index: Option<usize>,
    pub name: String,
    pub service_type: String,
    pub upstream_base_url: String,
    pub upstream_api_key: String,
    #[serde(default)]
    pub model_mapping: Vec<CodexProxyModelMapping>,
    #[serde(default)]
    pub insecure_skip_verify: bool,
    #[serde(default)]
    pub low_quality: bool,
    #[serde(default = "default_true")]
    pub auto_blacklist_balance: bool,
    #[serde(default = "default_true")]
    pub normalize_metadata_user_id: bool,
    #[serde(default)]
    pub normalize_nonstandard_chat_roles: bool,
    #[serde(default)]
    pub codex_tool_compat: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexProxyUpsertChannelResult {
    pub created: bool,
    pub channel_index: Option<usize>,
    pub codex_base_url: String,
    pub proxy_access_key: String,
    pub route_prefix: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexProxyResponsesChannel {
    pub index: usize,
    pub name: String,
    pub route_prefix: String,
    pub service_type: String,
    pub upstream_base_url: String,
    pub upstream_api_key: String,
    pub model_mapping: Vec<CodexProxyModelMapping>,
    pub insecure_skip_verify: bool,
    pub low_quality: bool,
    pub auto_blacklist_balance: bool,
    pub normalize_metadata_user_id: bool,
    pub normalize_nonstandard_chat_roles: bool,
    pub codex_tool_compat: bool,
    pub status: String,
}

fn default_true() -> bool {
    true
}

fn config_path() -> Result<PathBuf, String> {
    Ok(config::get_data_dir()?.join(CODEX_PROXY_CONFIG_FILE))
}

fn legacy_config_path() -> Result<PathBuf, String> {
    Ok(config::get_data_dir()?.join(LEGACY_CCX_GATEWAY_CONFIG_FILE))
}

fn runtime_dir() -> Result<PathBuf, String> {
    Ok(config::get_data_dir()?.join("codex_proxy"))
}

fn legacy_runtime_dir() -> Result<PathBuf, String> {
    Ok(config::get_data_dir()?.join("ccx_gateway"))
}

fn migrate_legacy_config_if_needed() -> Result<(), String> {
    let path = config_path()?;
    if path.exists() {
        return Ok(());
    }
    let legacy_path = legacy_config_path()?;
    if !legacy_path.exists() {
        return Ok(());
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|err| format!("创建兼容代理配置目录失败: {}", err))?;
    }
    fs::copy(&legacy_path, &path).map_err(|err| format!("迁移旧兼容代理配置失败: {}", err))?;
    logger::log_info(&format!(
        "[Codex Proxy] 已从旧配置迁移: {} -> {}",
        legacy_path.display(),
        path.display()
    ));
    Ok(())
}

fn migrate_legacy_runtime_if_needed() -> Result<(), String> {
    let target_config = runtime_dir()?.join(".config").join("config.json");
    if target_config.exists() {
        return Ok(());
    }
    let legacy_config = legacy_runtime_dir()?.join(".config").join("config.json");
    if !legacy_config.exists() {
        return Ok(());
    }
    if let Some(parent) = target_config.parent() {
        fs::create_dir_all(parent).map_err(|err| format!("创建兼容代理运行目录失败: {}", err))?;
    }
    fs::copy(&legacy_config, &target_config)
        .map_err(|err| format!("迁移旧兼容渠道配置失败: {}", err))?;
    logger::log_info(&format!(
        "[Codex Proxy] 已从旧运行配置迁移: {} -> {}",
        legacy_config.display(),
        target_config.display()
    ));
    Ok(())
}

fn migrate_legacy_state_if_needed() -> Result<(), String> {
    migrate_legacy_config_if_needed()?;
    migrate_legacy_runtime_if_needed()?;
    Ok(())
}

fn normalize_gateway_base_url(raw: &str) -> Result<String, String> {
    let trimmed = raw.trim().trim_end_matches('/');
    if trimmed.is_empty() {
        return Err("兼容代理地址不能为空".to_string());
    }
    let parsed = Url::parse(trimmed)
        .map_err(|_| "兼容代理地址格式无效，请输入完整的 http:// 或 https:// 地址".to_string())?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err("兼容代理地址仅支持 http 或 https".to_string());
    }
    Ok(trimmed.to_string())
}

fn normalize_required_url(raw: &str, label: &str) -> Result<String, String> {
    let trimmed = raw.trim().trim_end_matches('/');
    if trimmed.is_empty() {
        return Err(format!("{}不能为空", label));
    }
    let parsed = Url::parse(trimmed)
        .map_err(|_| format!("{}格式无效，请输入完整的 http:// 或 https:// 地址", label))?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err(format!("{}仅支持 http 或 https", label));
    }
    Ok(trimmed.to_string())
}

fn effective_admin_key(proxy_access_key: &str, admin_access_key: &str) -> Result<String, String> {
    let admin = admin_access_key.trim();
    if !admin.is_empty() {
        return Ok(admin.to_string());
    }
    let proxy = proxy_access_key.trim();
    if proxy.is_empty() {
        return Err("代理访问密钥不能为空".to_string());
    }
    Ok(proxy.to_string())
}

fn proxy_api_url(gateway_base_url: &str, path: &str) -> Result<String, String> {
    let base = normalize_gateway_base_url(gateway_base_url)?;
    Ok(format!("{}{}", base, path))
}

fn sanitize_route_prefix(raw: &str) -> String {
    let mut prefix = String::new();
    let mut last_was_dash = false;
    for ch in raw.trim().chars() {
        if ch.is_ascii_alphanumeric() {
            prefix.push(ch.to_ascii_lowercase());
            last_was_dash = false;
        } else if matches!(ch, '-' | '_' | ' ' | '.') && !prefix.is_empty() && !last_was_dash {
            prefix.push('-');
            last_was_dash = true;
        }
    }
    while prefix.ends_with('-') {
        prefix.pop();
    }
    if prefix.is_empty() {
        "channel".to_string()
    } else {
        prefix
    }
}

fn codex_channel_base_url(gateway_base_url: &str, route_prefix: &str) -> Result<String, String> {
    let base = normalize_gateway_base_url(gateway_base_url)?;
    let prefix = sanitize_route_prefix(route_prefix);
    Ok(format!("{}/{}/v1", base, prefix))
}

fn config_with_normalized_defaults(mut cfg: CodexProxyGatewayConfig) -> CodexProxyGatewayConfig {
    cfg.gateway_base_url = normalize_gateway_base_url(&cfg.gateway_base_url)
        .unwrap_or_else(|_| DEFAULT_GATEWAY_BASE_URL.to_string());
    cfg.proxy_access_key = cfg.proxy_access_key.trim().to_string();
    cfg.admin_access_key = cfg.admin_access_key.trim().to_string();
    cfg.binary_path = cfg.binary_path.trim().to_string();
    cfg
}

pub fn load_config() -> Result<CodexProxyGatewayConfig, String> {
    migrate_legacy_state_if_needed()?;
    let path = config_path()?;
    if !path.exists() {
        return Ok(CodexProxyGatewayConfig::default());
    }
    let content =
        fs::read_to_string(&path).map_err(|err| format!("读取兼容代理配置失败: {}", err))?;
    let cfg = serde_json::from_str::<CodexProxyGatewayConfig>(&content)
        .map_err(|err| format!("解析兼容代理配置失败: {}", err))?;
    Ok(config_with_normalized_defaults(cfg))
}

pub fn save_config(cfg: CodexProxyGatewayConfig) -> Result<CodexProxyGatewayConfig, String> {
    migrate_legacy_runtime_if_needed()?;
    let cfg = config_with_normalized_defaults(cfg);
    normalize_gateway_base_url(&cfg.gateway_base_url)?;
    let path = config_path()?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|err| format!("创建 Codex Proxy 配置目录失败: {}", err))?;
    }
    let content = serde_json::to_string_pretty(&cfg)
        .map_err(|err| format!("序列化 兼容代理配置失败: {}", err))?;
    atomic_write::write_string_atomic(&path, &content)
        .map_err(|err| format!("写入兼容代理配置失败: {}", err))?;
    Ok(cfg)
}

pub async fn health_check(
    gateway_base_url: Option<String>,
) -> Result<CodexProxyGatewayHealth, String> {
    let fallback = load_config().unwrap_or_default().gateway_base_url;
    let base = normalize_gateway_base_url(gateway_base_url.as_deref().unwrap_or(&fallback))?;
    let url = format!("{}/health", base);
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(3))
        .no_proxy()
        .build()
        .map_err(|err| format!("创建兼容代理健康检查客户端失败: {}", err))?;

    match client.get(&url).send().await {
        Ok(response) => {
            let status = response.status().as_u16();
            Ok(CodexProxyGatewayHealth {
                running: response.status().is_success(),
                gateway_base_url: base,
                status: Some(status),
                message: if response.status().is_success() {
                    "兼容代理运行正常".to_string()
                } else {
                    format!("兼容代理响应异常: HTTP {}", status)
                },
            })
        }
        Err(err) => Ok(CodexProxyGatewayHealth {
            running: false,
            gateway_base_url: base,
            status: None,
            message: format!("无法连接兼容代理: {}", err),
        }),
    }
}

pub async fn start_gateway() -> Result<CodexProxyGatewayHealth, String> {
    let cfg = load_config()?;
    let base = normalize_gateway_base_url(&cfg.gateway_base_url)?;
    let current = health_check(Some(base.clone())).await?;
    if current.running {
        return Ok(current);
    }

    let binary_path = resolve_gateway_binary(&cfg)?;

    let parsed = Url::parse(&base).map_err(|err| format!("解析兼容代理地址失败: {}", err))?;
    let port = parsed.port_or_known_default().unwrap_or(53000);
    migrate_legacy_runtime_if_needed()?;
    let runtime_dir = runtime_dir()?;
    fs::create_dir_all(&runtime_dir)
        .map_err(|err| format!("创建 Codex Proxy 运行目录失败: {}", err))?;

    let mut cmd = Command::new(&binary_path);
    cmd.current_dir(&runtime_dir)
        .env("PORT", port.to_string())
        .env("ENABLE_WEB_UI", "false")
        .env("APP_UI_LANGUAGE", "zh-CN")
        .env("PROXY_ACCESS_KEY", cfg.proxy_access_key.trim())
        .env("ADMIN_ACCESS_KEY", cfg.admin_access_key.trim())
        .stdout(Stdio::null())
        .stderr(Stdio::null());

    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }

    cmd.spawn()
        .map_err(|err| format!("启动兼容代理失败: {}", err))?;
    logger::log_info(&format!(
        "[Codex Proxy] 已尝试启动 sidecar: {}",
        binary_path.display()
    ));

    for _ in 0..20 {
        tokio::time::sleep(Duration::from_millis(300)).await;
        let health = health_check(Some(base.clone())).await?;
        if health.running {
            return Ok(health);
        }
    }

    health_check(Some(base)).await
}

/// 确保兼容代理网关正在运行：加载配置 → 健康检查 → 按需启动。
/// 供切号等场景调用，保证 Proxy 账号切换后网关可用。
pub async fn ensure_gateway_running() -> Result<CodexProxyGatewayHealth, String> {
    let cfg = load_config()?;
    let base_url = cfg.gateway_base_url.clone();
    let health = health_check(Some(base_url.clone())).await?;
    if health.running {
        return Ok(health);
    }
    if cfg.auto_start {
        start_gateway().await
    } else {
        Ok(CodexProxyGatewayHealth {
            running: false,
            gateway_base_url: base_url,
            status: None,
            message: "兼容代理未运行且未开启自动启动".to_string(),
        })
    }
}

pub fn ensure_gateway_for_current_proxy_account_on_startup() {
    tauri::async_runtime::spawn(async {
        let Some(account) = codex_account::get_current_account() else {
            logger::log_info("[Codex Proxy] 启动检查跳过：当前未选择 Codex 账号");
            return;
        };
        if !codex_account::is_codex_proxy_account(&account) {
            return;
        }

        match ensure_gateway_running().await {
            Ok(health) if health.running => {
                logger::log_info(&format!(
                    "[Codex Proxy] 启动检查完成，兼容代理网关已就绪: {}",
                    health.gateway_base_url
                ));
            }
            Ok(health) => {
                logger::log_warn(&format!(
                    "[Codex Proxy] 启动检查完成，但兼容代理网关未运行: {}",
                    health.message
                ));
            }
            Err(err) => {
                logger::log_warn(&format!(
                    "[Codex Proxy] 启动检查失败，无法确保兼容代理网关运行: {}",
                    err
                ));
            }
        }
    });
}

#[cfg(target_os = "windows")]
fn bundled_binary_name() -> &'static str {
    "codex-proxy.exe"
}

#[cfg(not(target_os = "windows"))]
fn bundled_binary_name() -> &'static str {
    "codex-proxy"
}

fn resolve_gateway_binary(cfg: &CodexProxyGatewayConfig) -> Result<PathBuf, String> {
    let configured = cfg.binary_path.trim();
    if !configured.is_empty() {
        let path = PathBuf::from(configured);
        if path.exists() {
            return Ok(path);
        }
        return Err(format!(
            "配置的 Codex Proxy 可执行文件不存在: {}",
            configured
        ));
    }

    let binary_name = bundled_binary_name();
    let mut candidates: Vec<PathBuf> = Vec::new();

    if let Some(app) = crate::get_app_handle() {
        if let Ok(resource_dir) = app.path().resource_dir() {
            candidates.push(resource_dir.join("codex-proxy").join(binary_name));
        }
    }

    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    candidates.push(
        manifest_dir
            .join("resources")
            .join("codex-proxy")
            .join(binary_name),
    );
    candidates.push(
        manifest_dir
            .join("sidecars")
            .join("codex-proxy")
            .join("dist")
            .join(binary_name),
    );

    for candidate in candidates {
        if candidate.exists() {
            return Ok(candidate);
        }
    }

    Err(
        "未找到内置 Codex Proxy。请先运行 npm run prepare-codex-proxy，或填写自定义可执行文件路径。"
            .to_string(),
    )
}

fn mapping_to_object(items: &[CodexProxyModelMapping]) -> Value {
    let mut map = serde_json::Map::new();
    let mut first_target: Option<Value> = None;
    for item in items {
        let source = item.source.trim();
        let target = item.target.trim();
        if !source.is_empty() && !target.is_empty() {
            let target_value = Value::String(target.to_string());
            if first_target.is_none() {
                first_target = Some(target_value.clone());
            }
            map.insert(source.to_string(), target_value);
        }
    }
    expand_codex_model_mapping_aliases(&mut map, first_target.as_ref());
    Value::Object(map)
}

fn explicit_mapping_to_object(items: &[CodexProxyModelMapping]) -> Value {
    let mut map = serde_json::Map::new();
    for item in items {
        let source = item.source.trim();
        let target = item.target.trim();
        if !source.is_empty() && !target.is_empty() {
            map.insert(source.to_string(), Value::String(target.to_string()));
        }
    }
    Value::Object(map)
}

fn expand_codex_model_mapping_aliases(
    map: &mut serde_json::Map<String, Value>,
    fallback_target: Option<&Value>,
) {
    const ALIAS_GROUPS: &[&[&str]] = &[
        &["gpt-5.5", "gpt-5.5-codex", "gpt-5.5-mini"],
        &["gpt-5.4", "gpt-5.4-codex", "gpt-5.4-mini"],
        &["gpt-5.3", "gpt-5.3-codex", "gpt-5.3-mini"],
        &["gpt-5", "gpt-5-codex", "gpt-5-mini"],
        &["gpt-4.1", "gpt-4.1-mini", "gpt-4.1-nano"],
    ];

    for aliases in ALIAS_GROUPS {
        let Some(target) = aliases
            .iter()
            .find_map(|alias| map.get(*alias))
            .or(fallback_target)
            .cloned()
        else {
            continue;
        };
        for alias in *aliases {
            map.entry((*alias).to_string())
                .or_insert_with(|| target.clone());
        }
    }
}

fn channel_payload(input: &CodexProxyResponsesChannelInput) -> Result<Value, String> {
    let name = input.name.trim();
    if name.is_empty() {
        return Err("渠道名称不能为空".to_string());
    }
    let service_type = input.service_type.trim();
    if !matches!(service_type, "openai" | "claude" | "gemini" | "responses") {
        return Err("服务类型仅支持 openai、claude、gemini、responses".to_string());
    }
    let upstream_base_url = normalize_required_url(&input.upstream_base_url, "上游 Base URL")?;
    let upstream_api_key = input.upstream_api_key.trim();
    if upstream_api_key.is_empty() {
        return Err("上游 API Key 不能为空".to_string());
    }

    Ok(json!({
        "name": name,
        "serviceType": service_type,
        "baseUrl": upstream_base_url,
        "routePrefix": sanitize_route_prefix(name),
        "apiKeys": [upstream_api_key],
        "modelMapping": mapping_to_object(&input.model_mapping),
        "modelMappingExplicit": explicit_mapping_to_object(&input.model_mapping),
        "insecureSkipVerify": input.insecure_skip_verify,
        "lowQuality": input.low_quality,
        "autoBlacklistBalance": input.auto_blacklist_balance,
        "normalizeMetadataUserId": input.normalize_metadata_user_id,
        "normalizeNonstandardChatRoles": input.normalize_nonstandard_chat_roles,
        "codexToolCompat": input.codex_tool_compat,
        "status": "active",
    }))
}

fn channel_index_at(value: &Value) -> Option<usize> {
    value
        .get("index")
        .and_then(Value::as_u64)
        .and_then(|item| usize::try_from(item).ok())
}

fn string_field(value: &Value, key: &str) -> String {
    value
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

fn bool_field(value: &Value, key: &str, default: bool) -> bool {
    value.get(key).and_then(Value::as_bool).unwrap_or(default)
}

fn mapping_from_object(value: Option<&Value>) -> Vec<CodexProxyModelMapping> {
    let Some(map) = value.and_then(Value::as_object) else {
        return Vec::new();
    };
    let mut rows = map
        .iter()
        .filter_map(|(source, target)| {
            let target = target.as_str()?.trim();
            let source = source.trim();
            if source.is_empty() || target.is_empty() {
                return None;
            }
            Some(CodexProxyModelMapping {
                source: source.to_string(),
                target: target.to_string(),
            })
        })
        .collect::<Vec<_>>();
    rows.sort_by(|a, b| a.source.cmp(&b.source));
    rows
}

fn compact_legacy_generated_mapping(value: Option<&Value>) -> Vec<CodexProxyModelMapping> {
    let Some(map) = value.and_then(Value::as_object) else {
        return Vec::new();
    };
    if map.is_empty() {
        return Vec::new();
    }

    let mut target_counts = std::collections::BTreeMap::<String, usize>::new();
    for target in map.values().filter_map(Value::as_str) {
        let target = target.trim();
        if !target.is_empty() {
            *target_counts.entry(target.to_string()).or_default() += 1;
        }
    }
    let majority_target = target_counts
        .into_iter()
        .max_by_key(|(_, count)| *count)
        .map(|(target, _)| target);

    const ALIAS_GROUPS: &[&[&str]] = &[
        &["gpt-5.5", "gpt-5.5-codex", "gpt-5.5-mini"],
        &["gpt-5.4", "gpt-5.4-codex", "gpt-5.4-mini"],
        &["gpt-5.3", "gpt-5.3-codex", "gpt-5.3-mini"],
        &["gpt-5", "gpt-5-codex", "gpt-5-mini"],
        &["gpt-4.1", "gpt-4.1-mini", "gpt-4.1-nano"],
    ];

    let mut rows = Vec::<CodexProxyModelMapping>::new();
    let mut kept_majority_canonical = false;
    let mut consumed = std::collections::BTreeSet::<String>::new();

    for aliases in ALIAS_GROUPS {
        for alias in *aliases {
            consumed.insert((*alias).to_string());
        }
        let canonical = aliases[0];
        let Some(canonical_target) = map
            .get(canonical)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|target| !target.is_empty())
        else {
            continue;
        };

        let alias_target_differs = aliases.iter().skip(1).any(|alias| {
            map.get(*alias)
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|target| !target.is_empty())
                .map(|target| target != canonical_target)
                .unwrap_or(false)
        });
        let is_majority = majority_target.as_deref() == Some(canonical_target);

        if alias_target_differs || !is_majority || !kept_majority_canonical {
            rows.push(CodexProxyModelMapping {
                source: canonical.to_string(),
                target: canonical_target.to_string(),
            });
            if is_majority {
                kept_majority_canonical = true;
            }
        }
    }

    for (source, target) in map {
        let source = source.trim();
        if source.is_empty() || consumed.contains(source) {
            continue;
        }
        let Some(target) = target
            .as_str()
            .map(str::trim)
            .filter(|target| !target.is_empty())
        else {
            continue;
        };
        rows.push(CodexProxyModelMapping {
            source: source.to_string(),
            target: target.to_string(),
        });
    }

    rows.sort_by(|a, b| a.source.cmp(&b.source));
    rows
}

fn parse_channel(value: &Value, fallback_index: usize) -> CodexProxyResponsesChannel {
    let upstream_api_key = value
        .get("apiKeys")
        .and_then(Value::as_array)
        .and_then(|items| items.first())
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();

    CodexProxyResponsesChannel {
        index: channel_index_at(value).unwrap_or(fallback_index),
        name: string_field(value, "name"),
        route_prefix: string_field(value, "routePrefix"),
        service_type: string_field(value, "serviceType"),
        upstream_base_url: string_field(value, "baseUrl"),
        upstream_api_key,
        model_mapping: {
            let explicit = mapping_from_object(value.get("modelMappingExplicit"));
            if explicit.is_empty() {
                compact_legacy_generated_mapping(value.get("modelMapping"))
            } else {
                explicit
            }
        },
        insecure_skip_verify: bool_field(value, "insecureSkipVerify", false),
        low_quality: bool_field(value, "lowQuality", false),
        auto_blacklist_balance: bool_field(value, "autoBlacklistBalance", true),
        normalize_metadata_user_id: bool_field(value, "normalizeMetadataUserId", true),
        normalize_nonstandard_chat_roles: bool_field(value, "normalizeNonstandardChatRoles", true),
        codex_tool_compat: bool_field(value, "codexToolCompat", false),
        status: string_field(value, "status"),
    }
}

pub async fn list_responses_channels(
    gateway_base_url: Option<String>,
    proxy_access_key: Option<String>,
    admin_access_key: Option<String>,
) -> Result<Vec<CodexProxyResponsesChannel>, String> {
    let cfg = load_config().unwrap_or_default();
    let gateway_base_url =
        normalize_gateway_base_url(gateway_base_url.as_deref().unwrap_or(&cfg.gateway_base_url))?;
    let proxy_key = proxy_access_key
        .as_deref()
        .unwrap_or(&cfg.proxy_access_key)
        .trim()
        .to_string();
    let admin_key_input = admin_access_key
        .as_deref()
        .unwrap_or(&cfg.admin_access_key)
        .trim()
        .to_string();
    let admin_key = effective_admin_key(&proxy_key, &admin_key_input)?;
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(20))
        .no_proxy()
        .build()
        .map_err(|err| format!("创建兼容代理管理 API 客户端失败: {}", err))?;
    let list_url = proxy_api_url(&gateway_base_url, "/api/responses/channels")?;
    let response = client
        .get(&list_url)
        .header("x-api-key", &admin_key)
        .send()
        .await
        .map_err(|err| format!("连接兼容代理管理 API 失败: {}", err))?;
    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(format!("读取兼容渠道失败: HTTP {} {}", status, body));
    }
    let list_json: Value = response
        .json()
        .await
        .map_err(|err| format!("解析兼容渠道列表失败: {}", err))?;
    let channels = list_json
        .get("channels")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .enumerate()
                .map(|(index, channel)| parse_channel(channel, index))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    Ok(channels)
}

pub async fn upsert_responses_channel(
    input: CodexProxyResponsesChannelInput,
) -> Result<CodexProxyUpsertChannelResult, String> {
    let gateway_base_url = normalize_gateway_base_url(&input.gateway_base_url)?;
    let proxy_key = input.proxy_access_key.trim();
    if proxy_key.is_empty() {
        return Err("代理访问密钥不能为空".to_string());
    }
    let route_prefix = sanitize_route_prefix(&input.name);
    let admin_key = effective_admin_key(proxy_key, &input.admin_access_key)?;
    let payload = channel_payload(&input)?;
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(20))
        .no_proxy()
        .build()
        .map_err(|err| format!("创建兼容代理管理 API 客户端失败: {}", err))?;

    let list_url = proxy_api_url(&gateway_base_url, "/api/responses/channels")?;
    let list_response = client
        .get(&list_url)
        .header("x-api-key", &admin_key)
        .send()
        .await
        .map_err(|err| format!("连接兼容代理管理 API 失败: {}", err))?;
    if !list_response.status().is_success() {
        let status = list_response.status();
        let body = list_response.text().await.unwrap_or_default();
        return Err(format!("读取兼容渠道失败: HTTP {} {}", status, body));
    }
    let _list_json: Value = list_response
        .json()
        .await
        .map_err(|err| format!("解析兼容渠道列表失败: {}", err))?;
    // 仅在编辑模式（channel_index 已指定）下更新已有渠道。
    // 新增模式（channel_index 为 None）直接走 POST，由 Go 后端负责 name 唯一性校验，
    // 避免按 name 静默匹配并覆盖已有渠道。
    let existing_index = input.channel_index;

    if let Some(index) = existing_index {
        let update_url = proxy_api_url(
            &gateway_base_url,
            &format!("/api/responses/channels/{}", index),
        )?;
        let response = client
            .put(&update_url)
            .header("x-api-key", &admin_key)
            .json(&payload)
            .send()
            .await
            .map_err(|err| format!("更新兼容渠道失败: {}", err))?;
        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(format!("更新兼容渠道失败: HTTP {} {}", status, body));
        }
        return Ok(CodexProxyUpsertChannelResult {
            created: false,
            channel_index: Some(index),
            codex_base_url: codex_channel_base_url(&gateway_base_url, &route_prefix)?,
            proxy_access_key: proxy_key.to_string(),
            route_prefix,
        });
    }

    let add_url = proxy_api_url(&gateway_base_url, "/api/responses/channels")?;
    let response = client
        .post(&add_url)
        .header("x-api-key", &admin_key)
        .json(&payload)
        .send()
        .await
        .map_err(|err| format!("新增兼容渠道失败: {}", err))?;
    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(format!("新增兼容渠道失败: HTTP {} {}", status, body));
    }

    Ok(CodexProxyUpsertChannelResult {
        created: true,
        channel_index: None,
        codex_base_url: codex_channel_base_url(&gateway_base_url, &route_prefix)?,
        proxy_access_key: proxy_key.to_string(),
        route_prefix,
    })
}

#[cfg(test)]
mod tests {
    use super::{mapping_to_object, CodexProxyModelMapping};

    #[test]
    fn mapping_to_object_expands_codex_model_aliases() {
        let value = mapping_to_object(&[CodexProxyModelMapping {
            source: "gpt-5.4".to_string(),
            target: "glm-5.1".to_string(),
        }]);
        let mapping = value.as_object().expect("mapping object");

        assert_eq!(
            mapping.get("gpt-5.4").and_then(|v| v.as_str()),
            Some("glm-5.1")
        );
        assert_eq!(
            mapping.get("gpt-5.4-mini").and_then(|v| v.as_str()),
            Some("glm-5.1")
        );
        assert_eq!(
            mapping.get("gpt-5.4-codex").and_then(|v| v.as_str()),
            Some("glm-5.1")
        );
    }

    #[test]
    fn mapping_to_object_routes_codex_aliases_to_first_mapping_target() {
        let value = mapping_to_object(&[CodexProxyModelMapping {
            source: "gpt-5.5".to_string(),
            target: "deepseek-v3.2".to_string(),
        }]);
        let mapping = value.as_object().expect("mapping object");

        assert_eq!(
            mapping.get("gpt-5.4-mini").and_then(|v| v.as_str()),
            Some("deepseek-v3.2")
        );
        assert_eq!(
            mapping.get("gpt-5-codex").and_then(|v| v.as_str()),
            Some("deepseek-v3.2")
        );
        assert_eq!(
            mapping.get("gpt-4.1-mini").and_then(|v| v.as_str()),
            Some("deepseek-v3.2")
        );
    }

    #[test]
    fn mapping_to_object_keeps_explicit_alias_override() {
        let value = mapping_to_object(&[
            CodexProxyModelMapping {
                source: "gpt-5.4".to_string(),
                target: "glm-5.1".to_string(),
            },
            CodexProxyModelMapping {
                source: "gpt-5.4-mini".to_string(),
                target: "glm-4.6".to_string(),
            },
        ]);
        let mapping = value.as_object().expect("mapping object");

        assert_eq!(
            mapping.get("gpt-5.4-mini").and_then(|v| v.as_str()),
            Some("glm-4.6")
        );
    }
}
