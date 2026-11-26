//! A 400/415 status alone must not replay an already-executed compressed turn.
use super::super::{bootstrap, contract::ServiceError};
use axum::{body::Bytes, http::header};
use futures_util::{stream, StreamExt};

pub(super) async fn inspect(
    response: reqwest::Response,
) -> Result<(reqwest::Response, bool), ServiceError> {
    let status = response.status();
    if response
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| {
            v.split(';')
                .next()
                .unwrap_or_default()
                .trim()
                .eq_ignore_ascii_case("text/event-stream")
        })
    {
        return Ok((response, false));
    }
    let response = super::decode_response(response)?;
    let version = response.version();
    let headers = response.headers().clone();
    let mut source = response.bytes_stream();
    let mut prefix = Vec::new();
    let mut tail = None;
    let mut complete = false;
    loop {
        match source.next().await {
            Some(Ok(bytes)) if prefix.len().saturating_add(bytes.len()) <= 64 * 1024 => {
                prefix.extend_from_slice(&bytes)
            }
            Some(Ok(bytes)) => {
                tail = Some(bytes);
                break;
            }
            Some(Err(error)) => {
                return Err(super::super::diagnostics::transport_failure(
                    502,
                    "upstream_stream_error",
                    "Unable to inspect the upstream compression rejection",
                    &error,
                ))
            }
            None => {
                complete = true;
                break;
            }
        }
    }
    let safe = complete
        && match serde_json::from_slice::<serde_json::Value>(&prefix) {
            Ok(value) => {
                !bootstrap::http_execution_observed(&value)
                    && (status.as_u16() == 415
                        || encoding_rejection(&value))
            }
            Err(_) => {
                let message = String::from_utf8_lossy(&prefix).to_ascii_lowercase();
                let message = message.trim_start();
                !message.starts_with("data:")
                    && !message.starts_with("event:")
                    && (status.as_u16() == 415 || compression_message(message))
            }
        };
    let replay = stream::iter(
        std::iter::once(Bytes::from(prefix))
            .chain(tail)
            .map(Ok::<_, reqwest::Error>),
    )
    .chain(source);
    let mut restored = axum::http::Response::new(reqwest::Body::wrap_stream(replay));
    *restored.status_mut() = status;
    *restored.version_mut() = version;
    *restored.headers_mut() = headers;
    Ok((reqwest::Response::from(restored), safe))
}

fn encoding_rejection(value: &serde_json::Value) -> bool {
    let error = value.get("error").unwrap_or(value);
    let code = error.get("code").and_then(serde_json::Value::as_str);
    let message = error.as_str().or_else(|| error["message"].as_str());
    matches!(
        code,
        Some("unsupported_content_encoding" | "unsupported_encoding" | "invalid_zstd" | "decompression_failed")
    ) || message.is_some_and(compression_message)
}

fn compression_message(message: &str) -> bool {
    let message = message.trim().to_ascii_lowercase();
    [
        "unsupported content-encoding",
        "unsupported content encoding",
        "unsupported encoding",
        "failed to decompress",
        "invalid zstd",
    ]
    .iter()
    .any(|prefix| message.starts_with(prefix))
}
