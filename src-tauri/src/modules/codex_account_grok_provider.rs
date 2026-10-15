// Codex 账号模块：Grok(xAI) 供应商账号。
//
// 这类账号自身不带上游 API Key，上游凭据来自 Cockpit 已登录的 Grok 平台账号（OAuth）。
// 本地网关启动时把该 Grok 账号的访问令牌写成 sidecar 的 xai auth 文件，由 sidecar
// 内置的 Grok(xAI) 执行器完成上游请求，因此模型目录、混合模型路由与 API 服务的
// 行为与其它供应商账号一致。
//
// 通过 include! 保持原 modules::codex_account 作用域和私有调用关系。

use crate::models::grok::{GrokAccount, GrokAuthMode};
use crate::modules::grok_account;

/// Grok 供应商账号的 provider id（与前端预设保持一致）。
pub(crate) const GROK_PROVIDER_ID: &str = "grok";
pub(crate) const GROK_PROVIDER_NAME: &str = "Grok";
/// Grok CLI 聊天代理地址：OAuth（订阅）账号的默认上游。
pub(crate) const GROK_CLI_CHAT_PROXY_BASE_URL: &str = "https://cli-chat-proxy.grok.com/v1";
/// Grok 账号带入 Codex 的默认模型目录；用户可在「模型与能力」中继续增删。
pub(crate) const GROK_CODEX_MODELS: &[&str] = &["grok-4.6", "grok-4.5", "grok-4.3"];
/// 默认支持图片输入的 Grok 模型（与上游能力对齐，可被用户覆盖）。
pub(crate) const GROK_VISION_CODEX_MODELS: &[&str] = &["grok-4.6", "grok-4.5", "grok-4.3"];

/// 账号是否绑定 Grok 平台账号作为上游（即 Grok 供应商账号）。
pub fn is_grok_upstream_provider(account: &CodexAccount) -> bool {
    account.is_api_key_auth()
        && account
            .upstream_grok_account_id
            .as_deref()
            .map(str::trim)
            .is_some_and(|value| !value.is_empty())
}

/// Grok 供应商账号默认带入的模型目录。
pub(crate) fn grok_provider_model_catalog() -> Vec<String> {
    GROK_CODEX_MODELS
        .iter()
        .map(|model| (*model).to_string())
        .collect()
}

pub(crate) fn grok_provider_vision_support() -> HashMap<String, bool> {
    GROK_VISION_CODEX_MODELS
        .iter()
        .map(|model| ((*model).to_string(), true))
        .collect()
}

/// Grok 供应商账号的存储 ID：同一个 Grok 账号重复添加时幂等。
pub(crate) fn build_grok_provider_account_id(grok_account_id: &str) -> String {
    format!(
        "codex_grok_{:x}",
        md5::compute(grok_account_id.trim().as_bytes())
    )
}

/// 读取该供应商账号绑定的 Grok 平台账号。
pub(crate) fn grok_upstream_account(account: &CodexAccount) -> Option<GrokAccount> {
    let grok_account_id = account.upstream_grok_account_id.as_deref()?.trim();
    if grok_account_id.is_empty() {
        return None;
    }
    grok_account::load_account(grok_account_id)
}

/// sidecar 中该账号的 xai auth 文件名。
pub(crate) fn grok_sidecar_auth_file_name(account_id: &str) -> String {
    format!("xai-{}.json", account_id.trim())
}

/// 生成 sidecar 使用的 xai OAuth auth 文件内容。
///
/// 只写入访问令牌，不写 refresh_token：Cockpit 保持 Token Authority，令牌轮换后
/// 由宿主回写该文件，sidecar 通过 auth 目录监听热更新，不需要重启网关。
///
/// `proxy_url` 为该账号绑定的代理：sidecar 读取 auth 文件里的 `proxy_url` 后，
/// 会为这个账号单独建立代理传输层，做到同进程内不同账号走不同出口。
pub(crate) fn grok_sidecar_auth_json(
    account: &CodexAccount,
    proxy_url: Option<&str>,
) -> Result<serde_json::Value, String> {
    let grok_account = grok_upstream_account(account)
        .ok_or_else(|| "Grok 供应商账号未绑定可用的 Grok 平台账号".to_string())?;
    if grok_account.auth_mode == GrokAuthMode::ApiKey {
        return Err("Grok 供应商账号只支持绑定 OAuth(订阅) 类型的 Grok 账号".to_string());
    }
    let access_token = grok_account.access_token.trim();
    if access_token.is_empty() {
        return Err("Grok 账号缺少访问令牌，请先在 Grok 页面重新授权".to_string());
    }
    let mut value = serde_json::json!({
        "type": "xai",
        "auth_kind": "oauth",
        "access_token": access_token,
        "token_type": grok_account
            .token_type
            .clone()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| "Bearer".to_string()),
        "email": grok_account.email.clone(),
        "base_url": GROK_CLI_CHAT_PROXY_BASE_URL,
        // OAuth 账号走 Grok CLI 聊天代理；官方 API 仅用于 compact 等端点。
        "using_api": false,
        "grok_account_id": grok_account.id.clone(),
    });
    if let Some(proxy_url) = proxy_url.map(str::trim).filter(|value| !value.is_empty()) {
        value["proxy_url"] = serde_json::json!(proxy_url);
    }
    if let Some(expires_at) = grok_account.expires_at {
        if let Some(expired) = grok_access_token_expired_at(expires_at) {
            value["expired"] = serde_json::json!(expired);
        }
    }
    Ok(value)
}

/// Grok 账号的过期时间在本地以 Unix 秒时间戳保存，这里转换成 xAI auth 使用的 RFC3339。
fn grok_access_token_expired_at(expires_at_seconds: i64) -> Option<String> {
    if expires_at_seconds <= 0 {
        return None;
    }
    chrono::DateTime::<chrono::Utc>::from_timestamp(expires_at_seconds, 0)
        .map(|value| value.to_rfc3339_opts(chrono::SecondsFormat::Secs, true))
}

/// 由 Grok 平台账号创建（或更新）一个 Codex 供应商账号。
pub fn upsert_grok_provider_account(
    grok_account_id: &str,
    grok_email: &str,
    model_catalog: Option<Vec<String>>,
    account_name: Option<String>,
) -> Result<CodexAccount, String> {
    let grok_account_id = grok_account_id.trim();
    if grok_account_id.is_empty() {
        return Err("请选择要绑定的 Grok 账号".to_string());
    }
    let grok_account = grok_account::load_account(grok_account_id)
        .ok_or_else(|| "Grok 账号不存在，请先在 Grok 页面登录".to_string())?;
    if grok_account.auth_mode == GrokAuthMode::ApiKey {
        return Err("Grok API Key 账号请改用「API Key」方式添加".to_string());
    }

    let account_id = build_grok_provider_account_id(grok_account_id);
    let email = match normalize_optional_value(Some(grok_email.to_string())) {
        Some(value) => value,
        None => grok_account.email.clone(),
    };
    let catalog = normalize_api_model_catalog(
        model_catalog
            .filter(|models| !models.is_empty())
            .unwrap_or_else(grok_provider_model_catalog),
    );
    let mut index = load_account_index();
    let mut account = load_account(&account_id).unwrap_or_else(|| {
        CodexAccount::new_api_key(
            account_id.clone(),
            email.clone(),
            String::new(),
            CodexApiProviderMode::Custom,
            Some(GROK_CLI_CHAT_PROXY_BASE_URL.to_string()),
            Some(GROK_PROVIDER_ID.to_string()),
            Some(GROK_PROVIDER_NAME.to_string()),
            catalog.clone(),
        )
    });

    account.auth_mode = CodexAuthMode::Apikey;
    account.email = email.clone();
    account.openai_api_key = None;
    account.api_provider_mode = CodexApiProviderMode::Custom;
    account.api_base_url = Some(GROK_CLI_CHAT_PROXY_BASE_URL.to_string());
    account.api_provider_id = Some(GROK_PROVIDER_ID.to_string());
    account.api_provider_name = Some(GROK_PROVIDER_NAME.to_string());
    account.api_model_catalog = catalog;
    account.api_wire_api = Some("responses".to_string());
    account.api_supports_websockets = false;
    account.api_supports_vision = true;
    account.api_model_vision_support = grok_provider_vision_support();
    account.api_vision_routing_model = None;
    account.api_sync_model_catalog_to_codex = false;
    account.plan_type = Some(API_KEY_LOGIN_PLAN_TYPE.to_string());
    account.upstream_grok_account_id = Some(grok_account_id.to_string());
    if let Some(name) = normalize_optional_value(account_name) {
        account.account_name = Some(name);
    }
    account.update_last_used();
    save_account_from_user_action(&mut account)?;

    if let Some(summary) = index.accounts.iter_mut().find(|item| item.id == account.id) {
        summary.email = account.email.clone();
        summary.plan_type = account.plan_type.clone();
        summary.subscription_active_until = account.subscription_active_until.clone();
        summary.last_used = account.last_used;
    } else {
        index.accounts.push(CodexAccountSummary {
            id: account.id.clone(),
            email: account.email.clone(),
            plan_type: account.plan_type.clone(),
            subscription_active_until: account.subscription_active_until.clone(),
            created_at: account.created_at,
            last_used: account.last_used,
        });
    }
    save_account_index(&index)?;

    logger::log_info(&format!(
        "Codex Grok 供应商账号已保存: account_id={}, grok_account_id={}",
        account.id, grok_account_id
    ));
    Ok(account)
}

#[cfg(test)]
mod grok_provider_tests {
    use super::*;

    fn grok_provider_account() -> CodexAccount {
        let mut account = CodexAccount::new_api_key(
            "codex_grok_test".to_string(),
            "grok@example.com".to_string(),
            String::new(),
            CodexApiProviderMode::Custom,
            Some(GROK_CLI_CHAT_PROXY_BASE_URL.to_string()),
            Some(GROK_PROVIDER_ID.to_string()),
            Some(GROK_PROVIDER_NAME.to_string()),
            grok_provider_model_catalog(),
        );
        account.upstream_grok_account_id = Some("grok-account-1".to_string());
        account
    }

    #[test]
    fn detects_only_bound_grok_provider_accounts() {
        let mut account = grok_provider_account();
        assert!(is_grok_upstream_provider(&account));

        account.upstream_grok_account_id = Some("   ".to_string());
        assert!(!is_grok_upstream_provider(&account));

        account.upstream_grok_account_id = None;
        assert!(!is_grok_upstream_provider(&account));
    }

    #[test]
    fn plain_api_key_accounts_are_not_grok_providers() {
        let account = CodexAccount::new_api_key(
            "codex_deepseek_test".to_string(),
            "deepseek@example.com".to_string(),
            "sk-test".to_string(),
            CodexApiProviderMode::Custom,
            Some("https://api.deepseek.com".to_string()),
            Some("deepseek".to_string()),
            Some("DeepSeek".to_string()),
            vec!["deepseek-flash".to_string()],
        );
        assert!(!is_grok_upstream_provider(&account));
    }

    #[test]
    fn grok_provider_account_id_is_stable_and_trimmed() {
        let first = build_grok_provider_account_id("grok-account-1");
        let second = build_grok_provider_account_id("  grok-account-1  ");
        assert_eq!(first, second);
        assert!(first.starts_with("codex_grok_"));
        assert_ne!(first, build_grok_provider_account_id("grok-account-2"));
    }

    #[test]
    fn grok_sidecar_auth_file_name_matches_account_scope_id() {
        assert_eq!(
            grok_sidecar_auth_file_name("codex_grok_test"),
            "xai-codex_grok_test.json"
        );
    }

    #[test]
    fn default_model_catalog_covers_grok_models() {
        let catalog = grok_provider_model_catalog();
        assert!(catalog.iter().any(|model| model == "grok-4.6"));
        assert!(catalog.iter().any(|model| model == "grok-4.5"));
        assert!(grok_provider_vision_support().values().all(|value| *value));
    }

    #[test]
    fn grok_access_token_expiry_uses_unix_seconds() {
        assert_eq!(
            grok_access_token_expired_at(1_893_456_000).as_deref(),
            Some("2030-01-01T00:00:00Z")
        );
        assert_eq!(grok_access_token_expired_at(0), None);
    }
}
