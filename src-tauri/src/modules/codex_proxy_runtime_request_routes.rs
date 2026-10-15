/// Read only the account's prepared stable entry; never start engines or perform I/O.
pub(crate) fn prepared_request_route_observer(
    account: &CodexAccount,
    proxy_url: &str,
) -> Option<codex_proxy_engine::RequestRouteObserver> {
    if super::codex_proxy_desktop_router::prepared_url(&account.id)
        .ok()?
        .as_str()
        != proxy_url
    {
        return None;
    }
    let mut observer = super::codex_proxy_desktop_router::request_route_observer(proxy_url)?;
    if let Some(binding) = codex_account_proxy::configured_url(account).ok()? {
        decorate_request_route_observer(&mut observer, binding.as_ref());
    }
    Some(observer)
}

pub(crate) fn decorate_request_route_observer(
    observer: &mut codex_proxy_engine::RequestRouteObserver,
    binding: &str,
) {
    if let Some(summary) = super::codex_proxy_catalog_binding::summary(binding) {
        observer.proxy_name = summary["name"].as_str().unwrap_or_default().to_string();
    } else if let Some((name, node)) = request_route_binding_label(binding) {
        observer.proxy_name = name.clone();
        if node {
            observer.node_names.insert("account-node".into(), name);
        }
    }
}

/// Labels contain no authentication, URL path, query, or raw subscription data.
fn request_route_binding_label(raw: &str) -> Option<(String, bool)> {
    let url = url::Url::parse(raw).ok()?;
    if matches!(url.scheme(), "http" | "https" | "socks5" | "socks5h") {
        return Some((
            format!(
                "{}://{}:{}",
                url.scheme(),
                url.host_str()?,
                url.port_or_known_default()?
            ),
            false,
        ));
    }
    let outbound = super::codex_proxy_node_parser::parse_node_link(raw).ok()?;
    let host = outbound["server"].as_str()?;
    let port = outbound["server_port"].as_u64()?;
    let label = url
        .fragment()
        .and_then(|value| urlencoding::decode(value).ok())
        .filter(|value| !value.trim().is_empty())
        .map(|value| value.into_owned())
        .unwrap_or_else(|| format!("{}://{}:{}", url.scheme(), host, port));
    Some((label, true))
}

#[cfg(test)]
mod request_route_label_tests {
    use super::*;

    #[test]
    fn proxy_request_route_labels_omit_credentials_and_query() {
        assert_eq!(
            request_route_binding_label(
                "http://user:secret@proxy.example:8080/?token=private#secret"
            ),
            Some(("http://proxy.example:8080".into(), false))
        );
        assert_eq!(
            request_route_binding_label("socks5://user:secret@[::1]:1080"),
            Some(("socks5://[::1]:1080".into(), false))
        );
        assert_eq!(
            request_route_binding_label("trojan://secret@proxy.example:443#Tokyo%2001"),
            Some(("Tokyo 01".into(), true))
        );
        assert_eq!(
            request_route_binding_label("trojan://secret@proxy.example:443"),
            Some(("trojan://proxy.example:443".into(), true))
        );
        assert_eq!(
            request_route_binding_label(
                "https://user:password@proxy.example/path?secret=query#secret"
            ),
            Some(("https://proxy.example:443".into(), false))
        );
        assert_eq!(request_route_binding_label("invalid secret binding"), None);
    }
}
