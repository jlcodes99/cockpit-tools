//! WebSocket handshakes use the same HTTP client as ordinary requests, including
//! the selected account's proxy, TLS settings and connection timeout.
use axum::http::{header, HeaderMap, Version};
use futures_util::StreamExt;
use tokio_tungstenite::{
    tungstenite::{
        handshake::{client::generate_key, derive_accept_key},
        protocol::{Role, WebSocketConfig},
    },
    WebSocketStream,
};

use super::{
    contract::{Route, ServiceError},
    lifecycle::{stopping_error, StopSignal},
};

#[cfg(test)]
#[path = "transport_unsent_dispatch_tests.rs"]
mod unsent_dispatch_tests;

pub(super) async fn connect_websocket(
    route: &Route,
    url: &str,
    headers: HeaderMap,
    max_message_bytes: usize,
    timeout: std::time::Duration,
    stop: StopSignal,
) -> Result<(WebSocketStream<reqwest::Upgraded>, HeaderMap), ServiceError> {
    connect_websocket_tracked(
        route,
        url,
        headers,
        max_message_bytes,
        timeout,
        stop,
        |_| {},
    )
    .await
}

/// Call the tracker only after local validation and when the network future is
/// actually polled. The caller can discard a proven unsent connect failure.
pub(super) async fn connect_websocket_tracked(
    route: &Route,
    url: &str,
    mut headers: HeaderMap,
    max_message_bytes: usize,
    timeout: std::time::Duration,
    mut stop: StopSignal,
    on_dispatch: impl FnOnce(&HeaderMap),
) -> Result<(WebSocketStream<reqwest::Upgraded>, HeaderMap), ServiceError> {
    if route.chatgpt_plan {
        return Err(ServiceError::new(
            400,
            "subscription_sharing_unsupported_capability",
            "ChatGPT plan accounts support HTTP Responses inference only",
        )
        .with_upstream_not_sent(true));
    }
    let url = url
        .strip_prefix("wss://")
        .map(|v| format!("https://{v}"))
        .or_else(|| url.strip_prefix("ws://").map(|v| format!("http://{v}")))
        .unwrap_or_else(|| url.to_owned());
    // Proxy credentials belong to the selected HTTP client, never the origin.
    headers.remove(header::PROXY_AUTHORIZATION);
    let deadline = tokio::time::Instant::now() + timeout;
    let key = generate_key();
    let expected = derive_accept_key(key.as_bytes());
    let request = route
        .client
        .get(url)
        .version(Version::HTTP_11)
        .headers(headers)
        .header(header::CONNECTION, "Upgrade")
        .header(header::UPGRADE, "websocket")
        .header(header::SEC_WEBSOCKET_VERSION, "13")
        .header(header::SEC_WEBSOCKET_KEY, key);
    let (client, request) = request.build_split();
    let request = request.map_err(|error| {
        super::diagnostics::transport_failure(
            502,
            "websocket_request_invalid",
            "Unable to prepare the upstream WebSocket request",
            &error,
        )
        .with_upstream_not_sent(true)
    })?;
    let response = tokio::select! {
        biased;
        _ = stop.wait() => return Err(stopping_error()),
        result = tokio::time::timeout_at(deadline, async {
            if tokio::time::Instant::now() >= deadline {
                return Err(ServiceError::new(504,"websocket_connect_timeout","Upstream WebSocket connection timed out"));
            }
            on_dispatch(request.headers());
            client.execute(request).await.map_err(|error| {
                let not_sent = error.is_connect() || error.is_builder();
                super::diagnostics::transport_failure(502,"websocket_connect_failed","Unable to establish the upstream WebSocket", &error)
                    .with_upstream_not_sent(not_sent)
            })
        }) => result
            .map_err(|_| ServiceError::new(504,"websocket_connect_timeout","Upstream WebSocket connection timed out"))?
            ?,
    };
    if response.status() != axum::http::StatusCode::SWITCHING_PROTOCOLS {
        let status = response.status().as_u16();
        let header_challenge = response
            .headers()
            .get("cf-mitigated")
            .is_some_and(|value| value == "challenge");
        let retry_after = super::events::retry_after_headers(response.headers());
        let mut bytes = Vec::new();
        let mut source = response.bytes_stream();
        loop {
            let next = tokio::select! {
                _=stop.wait()=>return Err(stopping_error()),
                result=tokio::time::timeout_at(deadline,source.next())=>result.map_err(|_|ServiceError::new(status,"websocket_upstream_rejected","Upstream error response timed out"))?,
            };
            match next {
                Some(Ok(chunk)) if bytes.len().saturating_add(chunk.len()) <= max_message_bytes => {
                    bytes.extend_from_slice(&chunk)
                }
                Some(Ok(_)) => {
                    return Err(ServiceError::new(
                        status,
                        "websocket_upstream_rejected",
                        "Upstream error response exceeds its size limit",
                    ))
                }
                Some(Err(error)) => {
                    return Err(super::diagnostics::transport_failure(
                        status,
                        "websocket_upstream_rejected",
                        "Unable to read the upstream error response",
                        &error,
                    ))
                }
                None => break,
            }
        }
        let body = serde_json::from_slice::<serde_json::Value>(&bytes).unwrap_or_else(|_| {
            serde_json::Value::String(String::from_utf8_lossy(&bytes).into_owned())
        });
        let (status, grok_wait) = if super::grok_errors::applies(route) {
            super::grok_errors::classify(status, &body)
        } else {
            (status, None)
        };
        let retry_after = retry_after.max(grok_wait);
        let (mut message, scope) =
            super::diagnostics::upstream_error_metadata_status(status, &body);
        if header_challenge {
            message = Some("cloudflare challenge".into());
        }
        return Err(ServiceError::new(
            status,
            if !header_challenge && super::upstream_errors::request_fault(status, &body) {
                super::upstream_errors::REQUEST_SCOPED
            } else if super::upstream_errors::model_unavailable(status, &body, &route.model) {
                "upstream_model_unavailable"
            } else {
                "websocket_upstream_rejected"
            },
            message.unwrap_or_else(|| "Upstream rejected the WebSocket connection".into()),
        )
        .with_retry_after(retry_after)
        .with_credential_scope(scope)
        .with_error_details(super::diagnostics::error_object(&body).clone()));
    }
    let headers = response.headers().clone();
    let token = |name: &str, value: &str| {
        headers
            .get(name)
            .and_then(|v| v.to_str().ok())
            .is_some_and(|v| v.split(',').any(|v| v.trim().eq_ignore_ascii_case(value)))
    };
    if headers
        .get(header::SEC_WEBSOCKET_ACCEPT)
        .and_then(|v| v.to_str().ok())
        != Some(expected.as_str())
        || !token("connection", "upgrade")
        || !token("upgrade", "websocket")
    {
        return Err(ServiceError::new(
            502,
            "invalid_websocket_handshake",
            "Upstream returned an invalid WebSocket handshake",
        ));
    }
    let upgraded = tokio::select! {
        _ = stop.wait() => return Err(stopping_error()),
        result = tokio::time::timeout_at(deadline, response.upgrade()) => result
            .map_err(|_| ServiceError::new(504,"websocket_connect_timeout","Upstream WebSocket upgrade timed out"))?
            .map_err(|error| super::diagnostics::transport_failure(502,"websocket_connect_failed","Unable to upgrade the upstream connection", &error))?,
    };
    let config = WebSocketConfig::default()
        .max_message_size(Some(max_message_bytes))
        .max_frame_size(Some(max_message_bytes));
    Ok((
        WebSocketStream::from_raw_socket(upgraded, Role::Client, Some(config)).await,
        headers,
    ))
}
