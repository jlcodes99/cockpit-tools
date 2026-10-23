//! A transport read failure is not evidence that the request exceeded a limit.
use std::error::Error;

use super::contract::ServiceError;

pub(super) async fn websocket_json(
    state: &super::server::AppState,
    text: axum::extract::ws::Utf8Bytes,
) -> Result<serde_json::Value, ServiceError> {
    let small = text.len() <= 64 * 1024;
    let parse = move || {
        serde_json::from_str(text.as_str())
            .map_err(|_| ServiceError::new(400, "invalid_json", "WebSocket message must be JSON"))
    };
    if small {
        parse()
    } else {
        super::blocking::run(state.stop.clone(), state.options.request_timeout, parse).await
    }
}

pub(super) fn read_error(error: axum::Error) -> ServiceError {
    let mut cause: Option<&(dyn Error + 'static)> = Some(&error);
    let mut message = String::new();
    while let Some(current) = cause {
        if current.is::<http_body_util::LengthLimitError>() {
            return ServiceError::new(
                413,
                "request_too_large",
                "Request body exceeds the configured size limit",
            );
        }
        let text = current.to_string();
        if !text.is_empty() && !message.contains(&text) {
            if !message.is_empty() {
                message.push_str(": ");
            }
            message.push_str(&text);
        }
        cause = current.source();
    }
    ServiceError::new(
        400,
        "request_body_read_error",
        super::diagnostics::sanitize_error_text(&message)
            .chars()
            .take(2048)
            .collect::<String>(),
    )
}
