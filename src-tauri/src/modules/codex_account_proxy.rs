//! Shared account-proxy policy. Provider/API-key routing must never consume this setting.
use crate::models::codex::{CodexAccount, CodexApiProviderMode};
use serde::{Serialize, Serializer};
use std::borrow::Cow;

/// All account responses/exports expose metadata, never the proxy credential.
pub fn serialize_summary<S: Serializer>(
    value: &Option<String>,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    let Some(raw) = value.as_deref().filter(|value| !value.trim().is_empty()) else {
        return serializer.serialize_none();
    };
    if let Some(summary) = super::codex_proxy_catalog_binding::summary(raw) {
        return summary.serialize(serializer);
    }
    let summary = match url::Url::parse(raw) {
        Ok(url) if matches!(url.scheme(), "http" | "https" | "socks5" | "socks5h") => {
            if normalize_direct_proxy(raw).is_err() {
                serde_json::json!({"protocol":"UNKNOWN"})
            } else {
                serde_json::json!({"protocol":url.scheme().to_uppercase(),"server":url.host_str(),"port":url.port_or_known_default()})
            }
        }
        Ok(url) => match crate::modules::codex_proxy_node_parser::parse_node_link(raw) {
            Ok(outbound) => {
                serde_json::json!({"protocol":url.scheme().to_uppercase(),"server":outbound["server"],"port":outbound["server_port"]})
            }
            Err(_) => serde_json::json!({"protocol":"UNKNOWN"}),
        },
        Err(_) => serde_json::json!({"protocol":"UNKNOWN"}),
    };
    summary.serialize(serializer)
}

/// The only raw serialization path is the encrypted account-file writer.
pub fn storage_value(account: &CodexAccount) -> Result<serde_json::Value, String> {
    let mut value = serde_json::to_value(account).map_err(|_| "PROXY_SAVE_FAILED")?;
    let object = value.as_object_mut().ok_or("PROXY_SAVE_FAILED")?;
    object.remove("egress_proxy");
    if let Some(raw) = account.egress_proxy_url.as_ref() {
        object.insert("egress_proxy_url".into(), serde_json::json!(raw));
    }
    Ok(value)
}

pub fn eligible(account: &CodexAccount) -> bool {
    !account.is_api_key_auth()
        && !account.is_agent_identity_auth()
        && !account.is_web_session_auth()
        && !crate::modules::codex_account::is_pending_oauth_account(account)
        && account.api_provider_mode != CodexApiProviderMode::Custom
        && account
            .api_provider_id
            .as_deref()
            .unwrap_or("")
            .trim()
            .is_empty()
        && account
            .upstream_grok_account_id
            .as_deref()
            .unwrap_or("")
            .trim()
            .is_empty()
}

/// 生效出口：账号自己的绑定优先于统一代理（全局）。
///
/// 统一代理只覆盖解析结果，不修改账号文件里的绑定；关闭统一代理后各账号立即
/// 回到原设置。返回值用 `Cow`，账号级绑定不需要额外分配。
pub fn configured_url(account: &CodexAccount) -> Result<Option<Cow<'_, str>>, String> {
    resolve_checked(account, crate::modules::codex_unified_proxy::effective_snapshot)
}

fn resolve_checked<'a>(
    account: &'a CodexAccount,
    unified: impl FnOnce() -> Result<Option<String>, String>,
) -> Result<Option<Cow<'a, str>>, String> {
    if !eligible(account) { return Ok(None); }
    if let Some(own) = resolve_effective(account, None) { return Ok(Some(own)); }
    Ok(resolve_effective(account, unified()?))
}

/// 解析优先级：账号自身绑定 > 统一代理 > 默认出口。抽成纯函数便于覆盖优先级矩阵。
pub(crate) fn resolve_effective<'a>(
    account: &'a CodexAccount,
    unified: Option<String>,
) -> Option<Cow<'a, str>> {
    let own = account
        .egress_proxy_url
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(Cow::Borrowed);
    own.or_else(|| unified.map(Cow::Owned))
}

/// 是否已有生效出口：账号自身绑定或统一代理。纯函数版本，便于启动路径与测试显式传入状态。
pub fn has_effective_proxy(account: &CodexAccount, unified_active: bool) -> bool {
    eligible(account)
        && (unified_active
            || account
                .egress_proxy_url
                .as_deref()
                .is_some_and(|value| !value.trim().is_empty()))
}

/// 只关心「有没有配置出口」的热路径用它，避免克隆统一代理快照。
pub fn has_configured_url(account: &CodexAccount) -> Result<bool, String> {
    Ok(configured_url(account)?.is_some())
}

/// Strict direct-proxy parsing; never include the supplied URL in an error.
/// Node protocols will be converted by the dedicated engine integration, not reqwest.
pub fn normalize_direct_proxy(input: &str) -> Result<String, String> {
    let input = input.trim();
    if input.len() > 8192 || input.chars().any(char::is_control) {
        return Err("PROXY_INVALID_URL".into());
    }
    let mut parsed = url::Url::parse(input).map_err(|_| "PROXY_INVALID_URL")?;
    if !matches!(parsed.scheme(), "http" | "https" | "socks5" | "socks5h") {
        return Err("PROXY_UNSUPPORTED_PROTOCOL".into());
    }
    if parsed.host_str().is_none()
        || parsed.port_or_known_default().is_none()
        || parsed.port_or_known_default() == Some(0)
        || !matches!(parsed.path(), "" | "/")
        || parsed.query().is_some()
    {
        return Err("PROXY_INVALID_URL".into());
    }
    parsed.set_fragment(None);
    reqwest::Proxy::all(parsed.as_str()).map_err(|_| "PROXY_INVALID_URL")?;
    Ok(parsed.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::codex::CodexTokens;

    fn account() -> CodexAccount {
        CodexAccount::new(
            "proxy-test".into(),
            "test@example.com".into(),
            CodexTokens {
                access_token: "access".into(),
                refresh_token: Some("refresh".into()),
                id_token: "id".into(),
            },
        )
    }

    #[test]
    fn account_responses_are_redacted_but_encrypted_storage_roundtrips() {
        let mut account = account();
        for value in [
            "http://user:TOPSECRET@proxy.example:8080",
            "trojan://TOPSECRET@proxy.example:443",
            "vmess://TOPSECRET",
        ] {
            account.egress_proxy_url = Some(value.into());
            let response = serde_json::to_value(&account).unwrap();
            assert!(response.get("egress_proxy_url").is_none());
            assert!(!response.to_string().contains("TOPSECRET"));
            assert!(response.get("egress_proxy").is_some());
            let persisted = storage_value(&account).unwrap();
            assert!(persisted.get("egress_proxy").is_none());
            let restored: CodexAccount = serde_json::from_value(persisted).unwrap();
            assert_eq!(restored.egress_proxy_url, account.egress_proxy_url);
            let imported_response: CodexAccount = serde_json::from_value(response).unwrap();
            assert_eq!(imported_response.egress_proxy_url, None);
        }
    }

    #[test]
    fn only_oauth_accounts_consume_proxy_settings() {
        let mut account = account();
        account.egress_proxy_url = Some("http://127.0.0.1:8080".into());
        for tier in ["FREE", "PLUS", "PRO", "TEAM"] {
            account.plan_type = Some(tier.into());
            assert!(eligible(&account));
        }
        account.api_provider_mode = CodexApiProviderMode::Custom;
        assert_eq!(configured_url(&account).unwrap(), None);
        account.api_provider_mode = CodexApiProviderMode::OpenaiBuiltin;
        account.upstream_grok_account_id = Some("grok".into());
        assert!(!eligible(&account));
        account.upstream_grok_account_id = None;
        account.token_source_mode = "chatgpt_web_session".into();
        assert!(!eligible(&account));
    }

    #[test]
    fn account_binding_wins_and_unified_is_fallback() {
        let mut account = account();
        account.egress_proxy_url = Some("http://127.0.0.1:8080".into());
        assert_eq!(
            resolve_effective(&account, None).as_deref(),
            Some("http://127.0.0.1:8080")
        );
        assert_eq!(
            resolve_effective(&account, Some("cockpit-proxy://shared".into())).as_deref(),
            Some("http://127.0.0.1:8080")
        );
        // 关闭统一代理后立即回到账号自己的绑定，账号数据从未被改写。
        assert_eq!(account.egress_proxy_url.as_deref(), Some("http://127.0.0.1:8080"));
        assert_eq!(resolve_effective(&account, None).as_deref(), Some("http://127.0.0.1:8080"));
    }

    #[test]
    fn shared_read_failures_never_become_an_unbound_account() {
        let mut account = account();
        assert_eq!(resolve_checked(&account, || Err("UNIFIED_PROXY_STORAGE".into())), Err("UNIFIED_PROXY_STORAGE".into()));
        account.egress_proxy_url = Some("http://127.0.0.1:8080".into());
        let own = resolve_checked(&account, || panic!("independent binding must not read shared state"));
        assert_eq!(own.unwrap().as_deref(), Some("http://127.0.0.1:8080"));
    }

    #[test]
    fn unified_binding_feeds_accounts_without_their_own_setting() {
        let account = account();
        assert_eq!(resolve_effective(&account, None), None);
        assert_eq!(
            resolve_effective(&account, Some("cockpit-proxy://shared".into())).as_deref(),
            Some("cockpit-proxy://shared")
        );
    }

    #[test]
    fn strict_proxy_parsing_supports_credentials_ipv6_and_remote_dns() {
        for value in [
            "http://user:p%40ss@127.0.0.1:8080",
            "https://proxy.example:443",
            "socks5://[::1]:1080",
            "socks5h://proxy.example:1080#node",
        ] {
            assert!(normalize_direct_proxy(value).is_ok());
        }
        for value in [
            "file:///etc/passwd",
            "http://host/path",
            "socks5://host",
            "http://host:0",
            "http://host:80?url=other",
            "https://user:secret@host/invalid",
        ] {
            let error = normalize_direct_proxy(value).unwrap_err();
            assert!(!error.contains("secret"));
        }
    }
}
