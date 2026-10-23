//! Public error envelopes shared by the native protocol adapters.
use axum::{body::Bytes, http::HeaderMap};
use serde_json::{json, Value};
use std::sync::LazyLock;

use super::contract::ServiceError;

pub(super) fn error_type(status: u16) -> &'static str {
    match status {
        401 => "authentication_error",
        403 => "permission_error",
        429 => "rate_limit_error",
        500..=599 => "server_error",
        _ => "invalid_request_error",
    }
}

pub(super) fn json_error(error: &ServiceError) -> Value {
    json!({"error": details(error, false)})
}

fn details(error: &ServiceError, stream: bool) -> Value {
    let parsed = serde_json::from_str::<Value>(&error.message).ok();
    let source = error.response_error.as_ref().or_else(|| {
        parsed.as_ref().and_then(|value| {
            value
                .pointer("/response/error")
                .or_else(|| value.get("error"))
        })
    });
    let mut result = source
        .filter(|value| value.is_object())
        .cloned()
        .unwrap_or_else(|| json!({}));
    let kind = if stream {
        if error.status >= 500 || error.status == 408 {
            "server_error"
        } else {
            "invalid_request_error"
        }
    } else {
        error_type(error.status)
    };
    let object = result.as_object_mut().unwrap();
    object.entry("type").or_insert_with(|| json!(kind));
    // Native codes carry continuation/isolation semantics and must remain available.
    object.entry("code").or_insert_with(|| json!(error.code));
    object
        .entry("message")
        .or_insert_with(|| json!(error.message));
    if stream {
        object.entry("param").or_insert(Value::Null);
    }
    sanitize(&mut result, 0);
    result
}

fn sensitive_key(key: &str) -> bool {
    let key = key.to_ascii_lowercase().replace('-', "_");
    matches!(
        key.as_str(),
        "authorization" | "secret" | "password" | "passwd" | "apikey" | "token" | "client_key"
    ) || ["_secret", "_password", "api_key", "_token"]
        .iter()
        .any(|suffix| key.ends_with(suffix))
}

fn public_text(text: &str) -> String {
    // Include bare token assignments, which the narrower diagnostic URL/auth
    // sanitizer does not recognize, matching Go's public stream-error boundary.
    static ASSIGNMENT: LazyLock<regex::Regex> = LazyLock::new(|| {
        regex::Regex::new(
        r#"(?i)((?:"?(?:api[_-]?key|access[_-]?token|token|authorization|secret)"?)\s*[=:]\s*"?)([^\s"&,;}]+)"#
    ).unwrap()
    });
    let text = ASSIGNMENT.replace_all(text, "${1}[REDACTED]");
    super::diagnostics::sanitize_error_text(&text)
        .replace("[redacted credential]", "[REDACTED]")
        .chars()
        .take(2048)
        .collect()
}

fn sanitize(value: &mut Value, depth: usize) {
    if depth > 32 {
        *value = Value::String("[redacted]".into());
        return;
    }
    match value {
        Value::Object(object) => {
            for (key, value) in object {
                if sensitive_key(key) {
                    *value = Value::String("[REDACTED]".into());
                } else {
                    sanitize(value, depth + 1);
                }
            }
        }
        Value::Array(array) => {
            for value in array {
                sanitize(value, depth + 1);
            }
        }
        Value::String(text) => *text = public_text(text),
        _ => {}
    }
}

pub(super) fn is_codex_client(headers: &HeaderMap) -> bool {
    let ua = headers
        .get("user-agent")
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default();
    if ua.starts_with("Codex Desktop/")
        || ua.starts_with("codex-tui/")
        || ua == "codex_cli_rs"
        || ua.starts_with("codex_cli_rs/")
    {
        return true;
    }
    let originator = headers
        .get("originator")
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    ["codex desktop", "codex-tui", "codex_cli_rs"]
        .iter()
        .any(|prefix| originator == *prefix || originator.starts_with(&format!("{prefix}/")))
}

pub(super) fn responses_event(error: &ServiceError, codex: bool, sequence: u64) -> Value {
    let detail = details(error, true);
    let mut event = if codex {
        json!({"type":"response.failed", "sequence_number":sequence, "response":{"status":"failed", "error":detail}})
    } else {
        json!({"type":"error", "sequence_number":sequence, "error":detail})
    };
    // Preserve native retry metadata used by existing clients and protocol adapters.
    event["status"] = json!(error.status);
    if let Some(delay) = error.retry_after {
        event["retry_after"] = json!(delay.as_secs_f64());
    }
    event
}

pub(super) fn event_sse(event: &Value) -> Bytes {
    Bytes::from(format!(
        "event: {}\ndata: {}\n\n",
        event["type"].as_str().unwrap_or("error"),
        event
    ))
}

pub(super) fn responses_sse(error: &ServiceError, codex: bool, sequence: u64) -> Bytes {
    event_sse(&responses_event(error, codex, sequence))
}

pub(super) fn event_failure(event: &Value, fallback: u16) -> ServiceError {
    let error = super::upstream_errors::chat_failure(event, fallback).unwrap_or_else(|| {
        ServiceError::new(
            fallback,
            "upstream_error",
            "Upstream returned a failed response",
        )
    });
    let detail = event
        .pointer("/response/error")
        .or_else(|| event.get("error"))
        .filter(|value| value.is_object())
        .cloned()
        .unwrap_or_else(|| super::upstream_errors::flat_error_details(event));
    error
        .with_error_details(detail)
        .with_retry_after(super::events::retry_after_event(event))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn public_errors_preserve_details_and_native_codes_without_credentials() {
        let error = ServiceError::new(429, "account_binding_cooling", "wait")
            .with_error_details(json!({"type":"rate_limit_error","code":"rate_limit_exceeded","message":"try later Bearer private-value", "param":"model", "retryable":true,"extra":{"access_token":"private-token","input_tokens":12},"nested":[{"password":"private-password"}]}));
        let body = json_error(&error);
        assert_eq!(body["error"]["param"], "model");
        assert_eq!(body["error"]["extra"]["input_tokens"], 12);
        assert_eq!(body["error"]["retryable"], true);
        assert!(!body.to_string().contains("private-"));
        for (status, kind) in [
            (401, "authentication_error"),
            (403, "permission_error"),
            (429, "rate_limit_error"),
            (502, "server_error"),
        ] {
            let body = json_error(&ServiceError::new(
                status,
                "account_binding_unavailable",
                "wait",
            ));
            assert_eq!(body["error"]["type"], kind);
            assert_eq!(body["error"]["code"], "account_binding_unavailable");
        }
    }

    #[test]
    fn nested_error_diagnostics_use_the_same_go_bound_as_messages() {
        let error = ServiceError::new(400,"invalid_request","invalid").with_error_details(json!({"message":"界".repeat(3000),"details":{"explanation":"界".repeat(500)},"extra":["界".repeat(3000)]}));
        let body = json_error(&error);
        assert_eq!(
            body["error"]["details"]["explanation"]
                .as_str()
                .unwrap()
                .chars()
                .count(),
            500
        );
        assert_eq!(
            body["error"]["message"].as_str().unwrap().chars().count(),
            2048
        );
        assert_eq!(
            body["error"]["extra"][0].as_str().unwrap().chars().count(),
            2048
        );
    }

    #[test]
    fn responses_failure_envelopes_match_client_and_keep_retry_metadata() {
        let error = ServiceError::new(429, "rate_limit_exceeded", "cooling")
            .with_retry_after(Some(std::time::Duration::from_millis(1500)));
        let generic = responses_event(&error, false, 7);
        let codex = responses_event(&error, true, 7);
        assert_eq!(generic["type"], "error");
        assert_eq!(codex["type"], "response.failed");
        assert_eq!(codex["response"]["status"], "failed");
        assert_eq!(codex["response"]["error"], generic["error"]);
        assert_eq!(codex["sequence_number"], 7);
        assert_eq!(codex["retry_after"], 1.5);
    }

    #[test]
    fn stream_errors_keep_the_real_cause_and_distinct_timeout_codes() {
        let eof = super::super::diagnostics::transport_failure(
            502,
            "upstream_stream_error",
            "Upstream stream disconnected",
            &std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                "unexpected EOF during chunk size line https://user:password@private.invalid/path?token=private-url token=private-token",
            ),
        );
        for (error, status, code, cause) in [
            (
                eof,
                502,
                "upstream_stream_error",
                "unexpected EOF during chunk size line",
            ),
            (
                ServiceError::new(
                    502,
                    "incomplete_upstream_stream",
                    "Upstream stream ended before a terminal response",
                ),
                502,
                "incomplete_upstream_stream",
                "ended before a terminal response",
            ),
            (
                ServiceError::new(
                    504,
                    "upstream_idle_timeout",
                    "Upstream stream stopped responding",
                ),
                504,
                "upstream_idle_timeout",
                "stopped responding",
            ),
            (
                super::super::retry::total_timeout(),
                504,
                "upstream_total_timeout",
                "total time limit",
            ),
        ] {
            for codex in [true, false] {
                let event = responses_event(&error, codex, 8);
                let details = if codex {
                    &event["response"]["error"]
                } else {
                    &event["error"]
                };
                assert_eq!(event["status"], status);
                assert_eq!(details["code"], code);
                assert!(details["message"].as_str().unwrap().contains(cause));
                let public = event.to_string();
                for hidden in [
                    "private.invalid",
                    "private-token",
                    "private-url",
                    "stream error:",
                    "stream disconnected before completion:",
                ] {
                    assert!(!public.contains(hidden), "{public}");
                }
            }
        }
    }

    #[test]
    fn codex_error_selection_uses_exact_supported_client_prefixes() {
        for ua in [
            "Codex Desktop/1",
            "codex-tui/1",
            "codex_cli_rs",
            "codex_cli_rs/1",
        ] {
            let headers =
                HeaderMap::from_iter([("user-agent".parse().unwrap(), ua.parse().unwrap())]);
            assert!(is_codex_client(&headers));
        }
        for ua in [
            "codex-other",
            "my-codex_cli_rs",
            "Codex Desktop impostor",
            "codex_cli_rsx",
        ] {
            let headers =
                HeaderMap::from_iter([("user-agent".parse().unwrap(), ua.parse().unwrap())]);
            assert!(!is_codex_client(&headers));
        }
        let headers = HeaderMap::from_iter([(
            "originator".parse().unwrap(),
            " CODEX-TUI/1 ".parse().unwrap(),
        )]);
        assert!(is_codex_client(&headers));
    }
}
