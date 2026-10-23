//! Bounded local failure details. Never retain URLs, credentials or authorization values.
use super::super::contract::{ServiceError, TransportFailureKind};
use regex::Regex;
use serde_json::Value;
use std::{error::Error, sync::LazyLock};

pub(crate) fn is_cloudflare_challenge(message: &str) -> bool {
    let lower = message.to_ascii_lowercase();
    lower.contains("challenge-platform")
        || lower.contains("cf-mitigated")
        || lower.contains("cloudflare challenge")
        || (lower.contains("cloudflare") && lower.contains("<html"))
}

pub(crate) fn sanitize_error_text(message: &str) -> String {
    static URL: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r#"(?i)\b[a-z][a-z0-9+.-]*://[^\s"'<>]+"#).unwrap());
    static AUTH: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r#"(?i)\b(?:bearer|basic)\s+[^\s,;]+|\b(?:authorization|api[-_]?key|access[-_]?token|refresh[-_]?token|id[-_]?token|token|password|passwd|secret)\s*["']?\s*[:=]\s*(?:"[^"]*"|'[^']*'|(?:(?:bearer|basic)\s+)?[^\s,;}]+)|\bsk-[A-Za-z0-9_-]+"# ).unwrap()
    });
    let bounded: String = message
        .chars()
        .take(8192)
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    let without_urls = URL.replace_all(&bounded, "[redacted URL]");
    AUTH.replace_all(&without_urls, "[redacted credential]")
        .chars()
        .take(4096)
        .collect()
}

pub(crate) fn transport_failure(
    status: u16,
    code: &'static str,
    context: &str,
    error: &(dyn Error + 'static),
) -> ServiceError {
    let mut details = Vec::new();
    let mut current = Some(error);
    let mut timed_out = false;
    let mut tls_decrypt = false;
    // Bootstrap can rewrap a reqwest body before canonical adaptation. Its
    // real TLS cause sits deeper than eight wrappers; still bound traversal
    // so a malformed/cyclic Error implementation cannot block the request.
    for _ in 0..32 {
        let Some(error) = current else {
            break;
        };
        // Canonical stream adapters wrap this error in io::Error. Keep its
        // classification instead of stringifying it into an untyped 502.
        if let Some(error) = error.downcast_ref::<ServiceError>() {
            return error.clone();
        }
        tls_decrypt |= matches!(
            error.downcast_ref::<rustls::Error>(),
            Some(rustls::Error::DecryptError)
        );
        let detail = sanitize_error_text(&error.to_string());
        timed_out |= error
            .downcast_ref::<std::io::Error>()
            .is_some_and(|error| error.kind() == std::io::ErrorKind::TimedOut)
            || error
                .downcast_ref::<reqwest::Error>()
                .is_some_and(reqwest::Error::is_timeout);
        if !detail.is_empty() && details.len() < 8 && !details.contains(&detail) {
            details.push(detail);
        }
        current = error
            .downcast_ref::<std::io::Error>()
            .and_then(std::io::Error::get_ref)
            .map(|inner| inner as &(dyn Error + 'static))
            .or_else(|| error.source());
    }
    // A transport deadline is different from EOF or a missing terminal event.
    // Classify only typed evidence; an upstream message mentioning timeouts is
    // not proof that our read timed out.
    let (status, code) = if code == "upstream_stream_error" && timed_out {
        (504, "upstream_timeout")
    } else {
        (status, code)
    };
    // Display implementations often include their source already, and io::Error
    // can expose it again through get_ref. Keep each cause once, omitting only
    // generic transport layers when a more specific cause is available.
    let message = concise_transport_details(&details);
    let mut error = ServiceError::new(status, code, format!("{context}: {message}"));
    error.transport_failure = tls_decrypt.then_some(TransportFailureKind::TlsDecrypt);
    error
}

fn concise_transport_details(details: &[String]) -> String {
    let is_layer = |text: &str| {
        matches!(
            text,
            "error decoding response body"
                | "request or response body error"
                | "error reading a body from connection"
                | "client error (Connect)"
                | "http2 error"
        )
    };
    let specific: Vec<_> = details
        .iter()
        .filter(|text| !is_layer(text))
        // A parent display may already contain its source verbatim.
        .filter(|text| {
            !details
                .iter()
                .any(|other| other.len() > text.len() && other.ends_with(&format!(": {text}")))
        })
        .map(String::as_str)
        .collect();
    if specific.is_empty() {
        details
            .last()
            .map(String::as_str)
            .unwrap_or("unknown transport error")
            .to_owned()
    } else {
        specific.join(": ")
    }
}

pub(crate) fn upstream_error_metadata(value: &Value) -> (Option<String>, bool) {
    upstream_error_metadata_status(0, value)
}

/// WebSocket errors can wrap the HTTP error envelope under `body`.
pub(crate) fn error_object(value: &Value) -> &Value {
    value
        .pointer("/response/error")
        .filter(|error| !error.is_null())
        .or_else(|| value.get("error").filter(|error| !error.is_null()))
        .or_else(|| {
            value
                .pointer("/body/error")
                .filter(|error| !error.is_null())
        })
        .or_else(|| value.get("detail").filter(|error| !error.is_null()))
        .unwrap_or(value)
}

pub(crate) fn upstream_error_metadata_status(status: u16, value: &Value) -> (Option<String>, bool) {
    let error = error_object(value);
    let code = error
        .get("code")
        .or_else(|| error.get("type"))
        .and_then(Value::as_str)
        .unwrap_or_default();
    let scope = (status == 401
        && [
            "/error/type",
            "/type",
            "/response/error/type",
            "/body/error/type",
            "/original_error_type",
        ]
        .iter()
        .any(|path| {
            value
                .pointer(path)
                .and_then(Value::as_str)
                .is_some_and(|kind| kind.trim().eq_ignore_ascii_case("authentication_error"))
        }))
        || matches!(
            code,
            "usage_limit_reached"
                | "subscription_sharing_usage_limit_exceeded"
                | "subscription_sharing_invalid_user"
                | "subscription_sharing_user_not_eligible"
                | "chatpass_v2_scope_not_authorized"
                | "chatpass_v2_invalid_authorization_context"
                | "insufficient_quota"
                | "invalid_grant"
                | "reauthorization_required"
                | "account_deactivated"
                | "account_disabled"
                | "account_suspended"
                | "organization_deactivated"
                | "quota_exceeded"
        );
    let message = error
        .get("message")
        .and_then(Value::as_str)
        .or_else(|| error.as_str())
        .map(sanitize_error_text);
    (message, scope)
}

pub(crate) fn failure_phase(code: &str) -> &'static str {
    match code {
        "upstream_unavailable"
        | "upstream_connect_timeout"
        | "websocket_connect_failed"
        | "websocket_connect_timeout" => "dial",
        "upstream_stream_error" | "incomplete_upstream_stream" | "upstream_timeout" => {
            "stream_read"
        }
        "upstream_idle_timeout" => "stream_idle",
        "upstream_total_timeout" => "total_timeout",
        "client_cancelled" | "client_disconnected" | "client_interrupted" => "client_cancelled",
        "identity_store_unavailable" | "identity_store_full" | "unknown_upstream_state" => {
            "identity"
        }
        _ => "upstream",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn tls_decrypt_requires_typed_evidence_and_survives_canonical_wrapping() {
        let mut raw = std::io::Error::other(rustls::Error::DecryptError);
        for _ in 0..12 {
            raw = std::io::Error::other(raw);
        }
        let failure = transport_failure(502, "upstream_stream_error", "Read failed", &raw);
        assert_eq!(
            failure.transport_failure,
            Some(TransportFailureKind::TlsDecrypt)
        );
        let wrapped = std::io::Error::other(failure.clone());
        let restored = transport_failure(503, "upstream_unavailable", "Another layer", &wrapped);
        assert_eq!(restored.transport_failure, failure.transport_failure);
        assert_eq!(
            (restored.status, restored.code),
            (502, "upstream_stream_error")
        );
        assert_eq!(restored.message, failure.message);
        for raw in [
            std::io::Error::other("cannot decrypt peer's message"),
            std::io::Error::new(std::io::ErrorKind::UnexpectedEof, "unexpected EOF"),
            std::io::Error::other(rustls::Error::InvalidCertificate(
                rustls::CertificateError::UnknownIssuer,
            )),
        ] {
            assert_eq!(
                transport_failure(502, "upstream_stream_error", "Read failed", &raw)
                    .transport_failure,
                None
            );
        }
        let remote = ServiceError::new(
            502,
            "upstream_stream_error",
            "cannot decrypt peer's message",
        );
        assert_eq!(
            transport_failure(502, "upstream_stream_error", "Read failed", &remote)
                .transport_failure,
            None
        );
    }

    #[test]
    fn parity22_runtime_wrapped_error_keeps_classification_and_public_details() {
        let wrapped = serde_json::json!({"type":"error","status":401,"body":{"error":{"code":"invalid_grant","type":"authentication_error","message":"fixture expired","param":"token"}}});
        let (message, scope) = upstream_error_metadata_status(401, &wrapped);
        assert_eq!(message.as_deref(), Some("fixture expired"));
        assert!(scope);
        let error = super::super::super::upstream_errors::chat_failure(&wrapped, 502).unwrap();
        assert_eq!(error.status, 401);
        assert!(error.credential_scope);
        assert_eq!(
            error.response_error.as_ref().unwrap()["code"],
            "invalid_grant"
        );
        assert_eq!(error.response_error.as_ref().unwrap()["param"], "token");
        let ordinary = serde_json::json!({"type":"error","status":429,"body":{"error":{"code":"rate_limit_exceeded","message":"fixture busy"}}});
        assert!(!upstream_error_metadata_status(429, &ordinary).1);
    }
    #[test]
    fn transport_timeouts_require_typed_evidence_and_preserve_the_cause() {
        for (kind, message, status, code) in [
            (
                std::io::ErrorKind::UnexpectedEof,
                "unexpected EOF during chunk size line",
                502,
                "upstream_stream_error",
            ),
            (
                std::io::ErrorKind::ConnectionReset,
                "connection reset after peer timeout",
                502,
                "upstream_stream_error",
            ),
            (
                std::io::ErrorKind::TimedOut,
                "socket read deadline elapsed",
                504,
                "upstream_timeout",
            ),
        ] {
            let error = std::io::Error::other(std::io::Error::new(kind, message));
            let failure = transport_failure(502, "upstream_stream_error", "Read failed", &error);
            assert_eq!((failure.status, failure.code), (status, code));
            assert!(failure.message.contains(message));
            assert_eq!(failure_phase(code), "stream_read");
        }
    }
    #[test]
    fn nested_transport_layers_keep_one_specific_cause() {
        let details = [
            "error decoding response body",
            "request or response body error",
            "error decoding response body",
            "error reading a body from connection",
            "cannot decrypt peer's message",
        ]
        .map(str::to_owned);
        assert_eq!(
            concise_transport_details(&details),
            "cannot decrypt peer's message"
        );
        let nested = ["socket read: unexpected EOF", "unexpected EOF"].map(str::to_owned);
        assert_eq!(
            concise_transport_details(&nested),
            "socket read: unexpected EOF"
        );
        let error = std::io::Error::other(std::io::Error::new(
            std::io::ErrorKind::UnexpectedEof,
            "unexpected EOF",
        ));
        let failure = transport_failure(
            502,
            "upstream_stream_error",
            "Upstream stream disconnected",
            &error,
        );
        assert_eq!(
            failure.message,
            "Upstream stream disconnected: unexpected EOF"
        );
        assert_eq!(failure.status, 502);
    }
    #[test]
    fn diagnostic_failures_are_bounded_and_do_not_leak_credentials_or_urls() {
        let detail = sanitize_error_text("connection reset https://user:password@example.invalid/path?api_key=secret Bearer token api_key=abc sk-secret");
        for secret in ["password", "example.invalid", "token", "abc", "sk-secret"] {
            assert!(!detail.contains(secret));
        }
        assert!(detail.contains("connection reset"));
        let quoted = sanitize_error_text(
            r#"{"api_key":"private-key","authorization":"Bearer private-token","refresh_token":"private-refresh"} authorization: Bearer unquoted-token token=bare-private-token passwd='private-password'"#,
        );
        for secret in [
            "private-key",
            "private-token",
            "private-refresh",
            "unquoted-token",
            "bare-private-token",
            "private-password",
        ] {
            assert!(!quoted.contains(secret), "{quoted}");
        }
        assert_eq!(
            sanitize_error_text(&"界".repeat(10000)).chars().count(),
            4096
        );
    }
    #[test]
    fn only_confirmed_account_error_codes_use_credential_scope() {
        for code in ["usage_limit_reached", "insufficient_quota", "invalid_grant"] {
            assert!(
                upstream_error_metadata(
                    &serde_json::json!({"error":{"code":code,"message":"limited"}})
                )
                .1
            );
        }
        assert!(
            !upstream_error_metadata(&serde_json::json!({"error":{"code":"rate_limit_exceeded"}}))
                .1
        );
    }
}
