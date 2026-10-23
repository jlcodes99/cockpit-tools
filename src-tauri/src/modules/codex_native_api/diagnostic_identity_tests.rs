use super::*;
use axum::http::{HeaderMap, HeaderValue};

#[test]
fn installation_metadata_is_observed_independently_and_never_captures_credentials() {
    let mut headers = HeaderMap::new();
    headers.insert("session-id", "session".parse().unwrap());
    headers.insert(
        "x-codex-installation-id",
        "original-installation".parse().unwrap(),
    );
    headers.insert("chatgpt-account-id", "original-workspace".parse().unwrap());
    headers.insert("authorization", "Bearer must-not-record".parse().unwrap());
    headers.insert("cookie", "must-not-record".parse().unwrap());
    let captured = capture_header_identity(&headers);
    assert!(captured.metadata_captured);
    assert_eq!(
        captured.installation.as_deref(),
        Some("original-installation")
    );
    assert_eq!(captured.account.as_deref(), Some("original-workspace"));
    assert!(!format!("{captured:?}").contains("must-not-record"));
    headers.insert(
        "x-codex-installation-id",
        HeaderValue::from_static("bad\tid"),
    );
    let invalid = capture_header_identity(&headers);
    assert!(invalid.captured);
    assert_eq!(invalid.session.as_deref(), Some("session"));
    assert!(!invalid.metadata_captured);
    assert!(invalid.installation.is_none());
    assert!(invalid.account.is_none());
    let absent = capture_header_identity(&HeaderMap::new());
    assert!(absent.metadata_captured);
    assert!(absent.installation.is_none());
}

#[test]
fn header_identity_uses_session_header_precedence_and_keeps_complete_ids() {
    let mut headers = HeaderMap::new();
    headers.insert("session_id", HeaderValue::from_static("legacy-session"));
    let alias = capture_header_identity(&headers);
    assert!(alias.captured);
    assert_eq!(alias.session.as_deref(), Some("legacy-session"));
    assert!(alias.thread.is_none());
    headers.insert("session-id", HeaderValue::from_static("original-session"));
    headers.insert("thread-id", HeaderValue::from_static("original-thread"));
    let identity = capture_header_identity(&headers);
    assert!(identity.captured);
    assert_eq!(identity.session.as_deref(), Some("original-session"));
    assert_eq!(identity.thread.as_deref(), Some("original-thread"));
    let longest = "x".repeat(4096);
    headers.insert("session-id", HeaderValue::from_str(&longest).unwrap());
    assert_eq!(
        capture_header_identity(&headers).session.as_deref(),
        Some(longest.as_str())
    );
    let unicode = "可见会话";
    headers.insert(
        "session-id",
        HeaderValue::from_bytes(unicode.as_bytes()).unwrap(),
    );
    assert_eq!(
        capture_header_identity(&headers).session.as_deref(),
        Some(unicode)
    );
}

#[test]
fn missing_identity_is_captured_without_inventing_ids_and_invalid_ids_are_not_truncated() {
    let missing = capture_header_identity(&HeaderMap::new());
    assert!(missing.captured);
    assert!(missing.session.is_none());
    assert!(missing.thread.is_none());
    let mut empty = HeaderMap::new();
    empty.insert("session-id", HeaderValue::from_static(""));
    let empty = capture_header_identity(&empty);
    assert!(empty.captured);
    assert!(empty.session.is_none());
    let mut fallback = HeaderMap::new();
    fallback.insert("session-id", HeaderValue::from_static(""));
    fallback.insert("session_id", HeaderValue::from_static("legacy-session"));
    assert_eq!(
        capture_header_identity(&fallback).session.as_deref(),
        Some("legacy-session")
    );
    for value in [
        HeaderValue::from_str(&"x".repeat(4097)).unwrap(),
        HeaderValue::from_static("bad\tid"),
        HeaderValue::from_bytes(b"bad\xff").unwrap(),
    ] {
        let mut headers = HeaderMap::new();
        headers.insert("session-id", value);
        headers.insert(
            "session_id",
            HeaderValue::from_static("valid-alias-must-not-replace-invalid-primary"),
        );
        headers.insert("thread-id", HeaderValue::from_static("valid-thread"));
        let identity = capture_header_identity(&headers);
        assert!(!identity.captured);
        assert!(identity.session.is_none());
        assert!(
            identity.thread.is_none(),
            "invalid identity must discard the whole pair"
        );
    }
}

#[test]
fn diagnostic_identity_fields_are_camel_case_and_old_records_remain_uncaptured() {
    let context = DiagnosticContext {
        client_identity_captured: true,
        client_session_id: Some("client-session".into()),
        client_thread_id: Some("client-thread".into()),
        routing_identity: Some(super::super::contract::RoutingIdentity {
            session_id: Some("canonical-body-session".into()),
            thread_id: Some("canonical-body-thread".into()),
            parent_thread_id: Some("canonical-body-parent".into()),
        }),
        attempts: vec![DiagnosticAttempt {
            upstream_identity_captured: true,
            upstream_session_id: Some("upstream-session".into()),
            upstream_thread_id: Some("upstream-thread".into()),
            ..Default::default()
        }],
        ..Default::default()
    };
    let mut value = serde_json::to_value(context).unwrap();
    assert_eq!(value["clientIdentityCaptured"], true);
    assert_eq!(value["clientSessionId"], "client-session");
    assert!(value.get("routingIdentity").is_none());
    assert!(!value.to_string().contains("canonical-body"));
    assert_eq!(value["attempts"][0]["upstreamThreadId"], "upstream-thread");
    for key in [
        "clientIdentityCaptured",
        "clientSessionId",
        "clientThreadId",
    ] {
        value.as_object_mut().unwrap().remove(key);
    }
    for key in [
        "upstreamIdentityCaptured",
        "upstreamSessionId",
        "upstreamThreadId",
    ] {
        value["attempts"][0].as_object_mut().unwrap().remove(key);
    }
    let old: DiagnosticContext = serde_json::from_value(value).unwrap();
    assert!(!old.client_identity_captured);
    assert!(old.client_session_id.is_none());
    assert!(old.client_thread_id.is_none());
    assert!(!old.attempts[0].upstream_identity_captured);
    assert!(old.attempts[0].upstream_session_id.is_none());
    assert!(old.attempts[0].upstream_thread_id.is_none());
}
