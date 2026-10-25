//! Explain local connection failures using the attempted route, without probing
//! the network or treating a transport failure as an account authorization error.
use super::contract::{Route, ServiceError};
use crate::modules::{config, i18n};
use std::error::Error;

#[derive(Clone, Copy)]
enum Cause {
    Connection,
    Timeout,
    ProxyAuth,
    LocalProxyUnavailable,
}

pub(super) fn request(route: &Route, source: &reqwest::Error) -> ServiceError {
    let error = super::diagnostics::transport_failure(
        502,
        "upstream_unavailable",
        "Unable to connect to the selected upstream",
        source,
    );
    // Invalid URLs/builders are configuration failures, not network evidence.
    if source.is_builder() {
        return error;
    }
    let cause = if source.is_timeout() {
        Cause::Timeout
    } else {
        classify(route, source)
    };
    explain(route, error, cause, &config::get_user_config().language)
}

pub(super) fn timeout(route: &Route) -> ServiceError {
    explain(
        route,
        ServiceError::new(504, "upstream_connect_timeout", ""),
        Cause::Timeout,
        &config::get_user_config().language,
    )
}

fn classify(route: &Route, source: &(dyn Error + 'static)) -> Cause {
    let proxy = route
        .proxy_url
        .as_deref()
        .and_then(|url| url::Url::parse(url).ok());
    let local_proxy = proxy
        .as_ref()
        .and_then(url::Url::host_str)
        .is_some_and(|host| {
            host.eq_ignore_ascii_case("localhost")
                || host
                    .trim_matches(['[', ']'])
                    .parse::<std::net::IpAddr>()
                    .is_ok_and(|ip| ip.is_loopback())
        });
    let mut current = Some(source);
    for _ in 0..32 {
        let Some(error) = current else { break };
        // These are CONNECT/SOCKS transport errors, never upstream response text.
        let detail = error.to_string().to_ascii_lowercase();
        if proxy.is_some()
            && (detail.contains("proxy authorization required")
                || detail.contains("socks error: credentials not accepted"))
        {
            return Cause::ProxyAuth;
        }
        if local_proxy
            && error
                .downcast_ref::<std::io::Error>()
                .is_some_and(|error| error.kind() == std::io::ErrorKind::ConnectionRefused)
        {
            return Cause::LocalProxyUnavailable;
        }
        current = error.source();
    }
    Cause::Connection
}

fn proxy_name(route: &Route) -> Option<String> {
    // proxy_url is the transport actually selected for this attempt; a stale
    // diagnostic label alone must not turn a direct request into a proxy error.
    let proxy = url::Url::parse(route.proxy_url.as_deref()?).ok()?;
    let name = route
        .proxy_route
        .as_ref()
        .filter(|value| value["kind"] == "proxy")
        .and_then(|value| value["name"].as_str())
        .map(str::trim)
        .filter(|name| !name.is_empty());
    // Endpoint-style labels omit userinfo, paths and queries. Never expose the
    // original proxy URL or a credential-bearing URL stored as a display name.
    if let Some(name) = name.filter(|name| !name.contains("://")) {
        return Some(
            super::diagnostics::sanitize_error_text(name)
                .chars()
                .take(160)
                .collect(),
        );
    }
    Some(format!(
        "{}:{}",
        proxy.host_str()?,
        proxy.port_or_known_default()?
    ))
}

fn explain(route: &Route, mut error: ServiceError, cause: Cause, locale: &str) -> ServiceError {
    let target = if url::Url::parse(&route.base_url)
        .ok()
        .and_then(|url| url.host_str().map(str::to_owned))
        .is_some_and(|host| host == "chatgpt.com" || host.ends_with(".chatgpt.com"))
    {
        "ChatGPT".to_owned()
    } else {
        i18n::translate(locale, "common.networkErrors.upstream", &[])
    };
    let proxy = proxy_name(route);
    let key = match (proxy.is_some(), cause) {
        (true, Cause::ProxyAuth) => "common.networkErrors.proxyAuth",
        (true, Cause::LocalProxyUnavailable) => "common.networkErrors.localProxyUnavailable",
        (true, Cause::Timeout) => "common.networkErrors.proxyTimeout",
        (true, Cause::Connection) => "common.networkErrors.proxyConnection",
        (false, Cause::Timeout) => "common.networkErrors.directTimeout",
        (false, _) => "common.networkErrors.directConnection",
    };
    error.message = i18n::translate(
        locale,
        key,
        &[
            ("proxy", proxy.as_deref().unwrap_or_default()),
            ("target", &target),
        ],
    );
    error
}

#[cfg(test)]
#[path = "connection_error_tests.rs"]
mod tests;
