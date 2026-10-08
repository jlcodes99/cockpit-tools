// apikey.fan 官方域名与旧域名兼容映射。

use reqwest::Url;

pub const APIKEY_FUN_PROVIDER_BASE_URL: &str = "https://api.apikey.fan/v1";
pub const APIKEY_FUN_LEGACY_PROVIDER_BASE_URL: &str = "https://api.apikey.fun/v1";

/// 把旧 `apikey.fun` 域名规范化为新的 `apikey.fan` 域名。
///
/// 仅处理已知的官方域名，返回 `None` 表示不需要迁移。
pub fn normalize_legacy_apikey_fun_url(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }

    let mut parsed = Url::parse(trimmed).ok()?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return None;
    }

    let mapped_host = match parsed.host_str()?.to_ascii_lowercase().as_str() {
        "apikey.fun" => "apikey.fan",
        "api.apikey.fun" => "api.apikey.fan",
        "slb.apikey.fun" => "slb.apikey.fan",
        _ => return None,
    };

    parsed.set_host(Some(mapped_host)).ok()?;
    parsed.set_fragment(None);
    Some(parsed.to_string().trim_end_matches('/').to_string())
}

#[cfg(test)]
mod tests {
    use super::normalize_legacy_apikey_fun_url;

    #[test]
    fn normalizes_known_legacy_hosts_and_preserves_paths() {
        assert_eq!(
            normalize_legacy_apikey_fun_url("https://api.apikey.fun/v1"),
            Some("https://api.apikey.fan/v1".to_string())
        );
        assert_eq!(
            normalize_legacy_apikey_fun_url("https://slb.apikey.fun"),
            Some("https://slb.apikey.fan".to_string())
        );
        assert_eq!(
            normalize_legacy_apikey_fun_url("https://apikey.fun/register?aff=cockpit"),
            Some("https://apikey.fan/register?aff=cockpit".to_string())
        );
    }

    #[test]
    fn leaves_new_and_unrelated_hosts_unchanged() {
        assert_eq!(
            normalize_legacy_apikey_fun_url("https://api.apikey.fan/v1"),
            None
        );
        assert_eq!(
            normalize_legacy_apikey_fun_url("https://relay.example.com/v1"),
            None
        );
    }
}
