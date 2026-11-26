//! Official rejection classification. Never infer replay safety from silence.
use super::{contract::OfficialFallbackFailure, diagnostics::AuthorizationFallbackTrace};
use serde_json::Value;

/// Older official sharing responses can carry only this quota message inside a
/// generic error envelope. Only callers handling an official response use this.
pub(super) fn normalize_sharing_quota_error(value: &mut Value) -> bool {
    let path = ["/response/error", "/error", "/body/error", "/detail"]
        .into_iter()
        .find(|path| value.pointer(path).is_some_and(|error| !error.is_null()));
    let Some(error) = (if let Some(path) = path {
        value.pointer_mut(path).and_then(Value::as_object_mut)
    } else if value.get("response").is_none()
        && value.get("type").is_none_or(|kind| {
            kind.is_null()
                || kind.as_str().is_some_and(|kind| {
                    matches!(
                        kind.trim(),
                        ""
                            | "error"
                            | "response.error"
                            | "invalid_request_error"
                            | "rate_limit_error"
                            | "server_error"
                            | "usage_limit_error"
                    )
                })
        })
    {
        value.as_object_mut()
    } else {
        None
    }) else {
        return false;
    };
    if ["code", "type"].iter().any(|field| {
        error.get(*field).is_some_and(|value| {
            !value.is_null()
                && !value.as_str().is_some_and(|raw| {
                    matches!(
                        raw.trim(),
                        ""
                            | "error"
                            | "response.error"
                            | "invalid_request_error"
                            | "rate_limit_error"
                            | "rate_limit_exceeded"
                            | "server_error"
                            | "usage_limit_error"
                            | "usage_limit_reached"
                            | "insufficient_quota"
                            | "quota_exceeded"
                            | "subscription_sharing_usage_limit_exceeded"
                    )
                })
        })
    }) {
        return false;
    }
    let Some(message) = error.get("message").and_then(Value::as_str) else {
        return false;
    };
    let trimmed = message.trim().trim_end_matches('.');
    let recognized = matches!(
        trimmed,
        "The ChatGPT user has reached their Subscription Sharing usage limit"
            | "The ChatGPT user has reached their Subscription Sharing usage limit. Ask the user to try again after their usage limit resets or use an API key instead"
    ) || trimmed
        .strip_prefix("The ChatGPT user has reached their Subscription Sharing usage limit")
        .is_some_and(|rest| rest.is_empty() || rest.starts_with('.'));
    if recognized {
        error.insert(
            "code".into(),
            Value::String("subscription_sharing_usage_limit_exceeded".into()),
        );
    }
    recognized
}

pub(super) fn code(value: &Value) -> Option<String> {
    let paths = [
        "/error/code",
        "/response/error/code",
        "/body/error/code",
        "/detail/code",
        "/code",
        "/error/type",
        "/response/error/type",
        "/body/error/type",
        "/detail/type",
        "/type",
    ];
    let official = paths
        .iter()
        .filter_map(|path| value.pointer(path).and_then(Value::as_str))
        .map(str::trim)
        .find(|code| super::chatgpt_plan_wire::error_status(code).is_some());
    if let Some(code) = official {
        return Some(code.into());
    }
    // Only the provider error message, never arbitrary request/output text.
    let message = super::diagnostics::error_object(value)
        .get("message")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim()
        .trim_end_matches('.')
        .to_ascii_lowercase();
    if matches!(
        message.as_str(),
        "user is not eligible for subscription sharing"
            | "this user is not eligible for subscription sharing"
            | "user is not eligible to use subscription sharing"
            | "this account is not eligible for subscription sharing"
            | "the chatgpt user is not eligible for subscription sharing"
    ) {
        return Some("subscription_sharing_user_not_eligible".into());
    }
    paths[..4]
        .iter()
        .filter_map(|path| value.pointer(path).and_then(Value::as_str))
        .map(str::trim)
        .find(|code| {
            !code.is_empty()
                && code.len() <= 128
                && code
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.'))
        })
        .map(str::to_owned)
}

/// Exact official authorization codes outrank a generic invalid_request_error
/// envelope. Capability errors are left to the explicit preflight gate.
pub(super) fn explicit_rejection(value: &Value) -> Option<(String, u16)> {
    let code = code(value)?;
    let status = super::chatgpt_plan_wire::error_status(&code)?;
    (status != 400).then_some((code, status))
}

pub(crate) fn reason(failure: &OfficialFallbackFailure) -> Option<&'static str> {
    match failure.code.as_str() {
        "eligibility_rejected"
        | "subscription_sharing_user_not_eligible"
        | "subscription_sharing_route_not_supported"
        | "chatpass_v2_scope_not_authorized"
        | "chatpass_v2_invalid_authorization_context" => Some("eligibility_rejected"),
        "authorization_failed"
        | "invalid_grant"
        | "invalid_token"
        | "subscription_sharing_invalid_user"
        | "chatgpt_plan_reauthorization_required"
        | "chatgpt_plan_permission_required"
        | "chatgpt_plan_verification_required" => Some("authorization_failed"),
        "capability_unsupported"
        | "subscription_sharing_unsupported_capability"
        | "model_not_found"
        | "model_not_supported"
        | "official_model_unavailable" => Some("capability_unsupported"),
        "official_throttled" | "subscription_sharing_usage_limit_exceeded" => {
            Some("official_throttled")
        }
        "official_unavailable"
        | "subscription_sharing_usage_unavailable"
        | "subscription_sharing_user_unavailable"
        | "upstream_connect_failed" => Some("official_unavailable"),
        // A generic parameter envelope never becomes an authorization failure.
        "invalid_request_error"
        | "invalid_argument"
        | "invalid_value"
        | "unsupported_value"
        | "invalid_encrypted_content"
        | "previous_response_not_found"
        | "context_length_exceeded"
        | "content_policy_violation" => None,
        "upstream_connect_timeout"
        | "upstream_idle_timeout"
        | "upstream_total_timeout"
        | "upstream_stream_error"
        | "incomplete_upstream_stream"
        | "upstream_timeout"
        | "client_cancelled"
        | "service_stopping" => None,
        _ => match failure.status {
            401 | 403 => Some("authorization_failed"),
            429 => Some("official_throttled"),
            500..=599 => Some("official_unavailable"),
            _ => None,
        },
    }
}

pub(super) fn local_unavailable(error: &super::contract::ServiceError) -> bool {
    matches!(error.status, 429 | 502 | 503 | 504)
        || matches!(
            error.code,
            "account_cooling"
                | "account_busy"
                | "local_fallback_unavailable"
                | "official_local_unavailable"
                | "no_available_account"
                | "model_not_supported"
                | "model_not_found"
                | "quota_exhausted"
                | "credentials_unavailable"
                | "credential_refresh_failed"
                | "credential_refresh_timeout"
                | "model_not_allowed"
        )
}

pub(super) fn trace(
    account_id: &str,
    failure: &OfficialFallbackFailure,
) -> AuthorizationFallbackTrace {
    AuthorizationFallbackTrace {
        trigger_stage: failure.trigger_stage.into(),
        reason: reason(failure).unwrap_or("official_unavailable").into(),
        from_account_id: account_id.into(),
        to_account_id: account_id.into(),
        from_route: "official".into(),
        to_route: "local_gateway".into(),
        status: (failure.status != 0).then_some(failure.status),
        error_code: (!failure.code.is_empty()).then(|| failure.code.clone()),
        error_message: failure
            .message
            .as_deref()
            .map(super::diagnostics::sanitize_error_text),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn sharing_quota_message_requires_an_exact_error_without_conflicting_code() {
        let message = "The ChatGPT user has reached their Subscription Sharing usage limit.";
        let full_message = "The ChatGPT user has reached their Subscription Sharing usage limit. Ask the user to try again after their usage limit resets or use an API key instead";
        let mut rejection = json!({"error":{"message":message,"type":"invalid_request_error"}});
        assert!(normalize_sharing_quota_error(&mut rejection));
        assert!(normalize_sharing_quota_error(&mut rejection));
        assert_eq!(explicit_rejection(&rejection), Some(("subscription_sharing_usage_limit_exceeded".into(), 429)));
        for mut recognized in [
            json!({"type":"response.failed","response":{"status":"failed","error":{"code":"server_error","message":full_message}}}),
            json!({"type":"response.failed","response":{"status":"failed","error":{"code":"rate_limit_exceeded","type":"rate_limit_error","message":full_message}}}),
            json!({"type":"error","code":null,"message":full_message,"param":null}),
        ] {
            assert!(normalize_sharing_quota_error(&mut recognized), "{recognized}");
            assert_eq!(
                explicit_rejection(&recognized),
                Some(("subscription_sharing_usage_limit_exceeded".into(), 429)),
                "{recognized}"
            );
        }
        for mut unrelated in [
            json!({"error":{"message":message,"code":"invalid_value"}}),
            json!({"error":{"message":format!("Prompt quoted: {message}")}}),
            json!({"type":"response.output_text.delta","delta":message}),
        ] {
            let before = unrelated.clone();
            assert!(!normalize_sharing_quota_error(&mut unrelated));
            assert_eq!(unrelated, before);
        }
    }

    #[test]
    fn precise_official_rejection_outweighs_generic_envelope() {
        let value = json!({"error":{"code":"subscription_sharing_user_not_eligible","type":"invalid_request_error"}});
        assert_eq!(
            explicit_rejection(&value),
            Some(("subscription_sharing_user_not_eligible".into(), 403))
        );
        assert!(!super::super::upstream_errors::request_fault(403, &value));
        assert_eq!(
            super::super::bootstrap::rejection(
                &json!({"type":"response.failed","response":{"error":value["error"]}})
            )
            .unwrap()
            .0,
            403
        );
        assert!(explicit_rejection(&json!({"error":{"code":"invalid_request_error"}})).is_none());
    }

    #[test]
    fn parameter_faults_and_ambiguous_timeouts_do_not_allow_fallback() {
        for (status, code) in [
            (403, "invalid_request_error"),
            (400, "invalid_argument"),
            (504, "upstream_connect_timeout"),
            (504, "upstream_idle_timeout"),
            (502, "upstream_stream_error"),
            (502, "incomplete_upstream_stream"),
        ] {
            let failure = OfficialFallbackFailure {
                trigger_stage: "upstream",
                status,
                code: code.into(),
                message: None,
                retry_after: None,
            };
            assert!(reason(&failure).is_none());
        }
    }

    #[test]
    fn original_source_preflight_failure_can_use_pool_but_caller_and_owner_changes_stop() {
        use super::super::contract::ServiceError;
        for (status, code) in [
            (401, "credentials_unavailable"),
            (401, "credential_refresh_failed"),
            (504, "credential_refresh_timeout"),
            (403, "model_not_allowed"),
        ] {
            assert!(
                local_unavailable(&ServiceError::new(status, code, "source unavailable")),
                "{code}"
            );
        }
        for (status, code) in [
            (401, "invalid_api_key"),
            (409, "official_fallback_authorization_changed"),
            (409, "official_fallback_owner_changed"),
            (403, "permission_denied"),
        ] {
            assert!(
                !local_unavailable(&ServiceError::new(status, code, "scope changed")),
                "{code}"
            );
        }
    }
}
