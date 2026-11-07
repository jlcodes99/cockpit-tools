//! Public Realtime uses the OpenAI API; native backend sideband stays on Codex.
use super::{invalid, Route, ServiceError};
use axum::http::{header, HeaderMap};

fn public_base(route: &Route) -> String {
    let base = route.base_url.trim_end_matches('/');
    if route.chatgpt
        && url::Url::parse(base).is_ok_and(|url| {
            url.host_str() == Some("chatgpt.com") && url.path() == "/backend-api/codex"
        })
    {
        "https://api.openai.com/v1".into()
    } else {
        // Explicit custom endpoints (including loopback fixtures) keep their root.
        base.into()
    }
}

pub(super) fn hangup_url(route: &Route, id: &str) -> String {
    format!("{}/realtime/calls/{id}/hangup", public_base(route))
}

pub(super) fn websocket_url(
    route: &Route,
    path: &str,
    call_id: Option<&str>,
    model: &str,
) -> Result<url::Url, ServiceError> {
    let backend = path.starts_with("/backend-api/codex/");
    let base = if backend {
        route.base_url.trim_end_matches('/').to_owned()
    } else {
        public_base(route)
    };
    let target = match call_id {
        Some(id) if backend => format!("{base}/{id}"),
        Some(id) if path.starts_with("/v1/live/") => format!("{base}/live/{id}"),
        Some(id) if path.starts_with("/v1/realtime/calls/") => {
            format!("{base}/realtime/calls/{id}")
        }
        Some(id) => format!("{base}/realtime?intent=quicksilver&call_id={id}"),
        None => format!("{base}/realtime"),
    };
    let mut url = url::Url::parse(&target).map_err(|_| invalid("Invalid upstream URL"))?;
    if call_id.is_none() {
        url.query_pairs_mut().append_pair("model", model.trim());
    }
    let scheme = match url.scheme() {
        "http" => "ws",
        "https" => "wss",
        "ws" => "ws",
        "wss" => "wss",
        _ => return Err(invalid("Invalid upstream URL")),
    };
    url.set_scheme(scheme)
        .map_err(|_| invalid("Invalid upstream URL"))?;
    Ok(url)
}

pub(super) fn offered_protocols(headers: &HeaderMap) -> Vec<String> {
    let mut protocols = Vec::new();
    for value in headers.get_all(header::SEC_WEBSOCKET_PROTOCOL) {
        let Ok(value) = value.to_str() else { continue };
        for protocol in value
            .split(',')
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            // Browser-local authentication tokens must never reach the upstream.
            if protocol
                .to_ascii_lowercase()
                .starts_with("openai-insecure-api-key.")
            {
                continue;
            }
            if !protocols.iter().any(|value| value == protocol) {
                protocols.push(protocol.to_owned());
            }
        }
    }
    protocols
}

pub(super) fn selected_protocol(
    offered: &[String],
    headers: &HeaderMap,
) -> Result<Option<String>, ServiceError> {
    let Some(selected) = headers.get(header::SEC_WEBSOCKET_PROTOCOL) else {
        return Ok(None);
    };
    let selected = selected
        .to_str()
        .ok()
        .map(str::trim)
        .filter(|selected| offered.iter().any(|offered| offered == selected))
        .ok_or_else(|| {
            ServiceError::new(
                502,
                "invalid_websocket_handshake",
                "Upstream selected an unoffered Realtime subprotocol",
            )
        })?;
    Ok(Some(selected.to_owned()))
}
