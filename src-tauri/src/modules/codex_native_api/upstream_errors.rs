//! Error envelopes and model availability, separate from transport and ownership.
use super::{contract::ServiceError, diagnostics::upstream_error_metadata, events::SseFrame};
use serde_json::{json, Value};

pub(crate) const REQUEST_SCOPED: &str = "upstream_request_scoped";

/// Mirrors the Go host's request-fault decision before credential penalties.
/// A status alone must not turn malformed prompts into authentication failures.
pub(crate) fn request_fault(status: u16, body: &Value) -> bool {
    if super::official_fallback::explicit_rejection(body).is_some() {
        return false;
    }
    let text = body
        .as_str()
        .map(str::to_owned)
        .unwrap_or_else(|| body.to_string())
        .to_ascii_lowercase();
    if matches!(status, 402 | 429)
        || (matches!(status, 400 | 401) && text.contains("invalid_grant"))
        || super::diagnostics::is_cloudflare_challenge(&text)
        || model_unavailable(status, body, "")
    {
        return false;
    }
    let identifiers = |field: &str| {
        [
            format!("/error/{field}"),
            format!("/{field}"),
            format!("/response/error/{field}"),
            format!("/body/error/{field}"),
            if field == "type" {
                "/original_error_type".into()
            } else {
                format!("/{field}")
            },
        ]
        .into_iter()
        .filter_map(|path| {
            body.pointer(&path)
                .and_then(Value::as_str)
                .map(|value| value.trim().to_ascii_lowercase())
        })
        .collect::<Vec<_>>()
    };
    let codes = identifiers("code");
    let types = identifiers("type");
    if status == 401 && types.iter().any(|kind| kind == "authentication_error") {
        return false;
    }
    if codes.iter().any(|code| {
        matches!(
            code.as_str(),
            "cyber_policy"
                | "context_length_exceeded"
                | "message_too_big"
                | "string_above_max_length"
                | "invalid_prompt"
                | "invalid_value"
                | "unsupported_value"
                | "invalid_request_error"
                | "previous_response_not_found"
        )
    }) || types.iter().any(|kind| {
        matches!(
            kind.as_str(),
            "invalid_request" | "invalid_request_error" | "bad_request_error" | "invalid_prompt"
        )
    }) {
        return true;
    }
    (text.contains("item with id")
        && text.contains("not found")
        && text.contains("items are not persisted when `store` is set to false"))
        || matches!(status, 400 | 409 | 413 | 422 | 499)
}

pub(crate) fn model_unavailable(status: u16, value: &Value, model: &str) -> bool {
    fn input_fault(value: &Value) -> bool {
        match value {
            Value::Object(fields) => fields.iter().any(|(key, value)| {
                matches!(key.as_str(), "code" | "type")
                    && value.as_str().is_some_and(|code| {
                        matches!(
                            code,
                            "invalid_encrypted_content"
                                | "thinking_signature_invalid"
                                | "previous_response_not_found"
                                | "context_length_exceeded"
                                | "context_too_large"
                                | "invalid_prompt"
                                | "invalid_argument"
                                | "invalid_value"
                                | "content_policy_violation"
                                | "cyber_policy"
                                | "unsupported_value"
                                | "message_too_big"
                                | "string_above_max_length"
                        )
                    })
                    || (value.is_object() || value.is_array()) && input_fault(value)
            }),
            Value::Array(values) => values.iter().any(input_fault),
            _ => false,
        }
    }
    if input_fault(value) {
        return false;
    }
    fn identifier(value: &str) -> bool {
        let value = value.trim().to_ascii_lowercase();
        let tail = if let Some((_, tail)) = value.rsplit_once('#') {
            tail
        } else {
            value
                .split('?')
                .next()
                .unwrap_or("")
                .trim_end_matches('/')
                .rsplit(['/', ':'])
                .next()
                .unwrap_or("")
        };
        matches!(
            tail.replace(['-', ' '], "_").as_str(),
            "model_not_found"
                | "model_not_found_error"
                | "unknown_model"
                | "model_does_not_exist"
                | "model_not_exist"
        )
    }
    fn missing_message(text: &str, model: &str) -> bool {
        let text = text
            .trim()
            .trim_end_matches([' ', '.', '!', ';'])
            .to_ascii_lowercase();
        if ["in request", "in body", "request body"]
            .iter()
            .any(|part| text.contains(part))
        {
            return false;
        }
        if text.replace('-', "_").contains("model_not_found")
            || text.replace('-', "_").contains("unknown_model")
        {
            return true;
        }
        for prefix in [
            "no such model",
            "unknown model",
            "the requested model",
            "requested model",
            "the model",
            "model",
        ] {
            let Some(rest) = text.strip_prefix(prefix) else {
                continue;
            };
            if !rest.is_empty() && !rest.starts_with([' ', ':']) {
                continue;
            }
            let rest = rest.trim_start_matches([' ', ':']);
            let rest = if model.is_empty() {
                rest
            } else {
                let model = model.to_ascii_lowercase();
                [
                    model.clone(),
                    format!("'{model}'"),
                    format!("\"{model}\""),
                    format!("`{model}`"),
                ]
                .iter()
                .find_map(|name| {
                    rest.strip_prefix(name)
                        .filter(|tail| tail.is_empty() || tail.starts_with([' ', ':', ',']))
                })
                .unwrap_or(rest)
                .trim_start_matches([' ', ':', ','])
            };
            if matches!(
                rest,
                "not found"
                    | "does not exist"
                    | "could not be found"
                    | "was not found"
                    | "doesn't exist"
                    | "not exist"
                    | "is unknown"
                    | "does not exist or you do not have access to it"
            ) || (rest.is_empty() && matches!(prefix, "no such model" | "unknown model"))
            {
                return true;
            }
        }
        false
    }
    fn structured(value: &Value, model: &str) -> bool {
        match value {
            Value::Array(items) => items.iter().any(|item| {
                structured(item, model) || item.as_str().is_some_and(|s| missing_message(s, model))
            }),
            Value::Object(fields) => {
                let exact_model = !model.is_empty()
                    && fields.values().filter_map(Value::as_str).any(|text| {
                        let text = text
                            .trim()
                            .trim_end_matches(['.', '!'])
                            .to_ascii_lowercase();
                        let model = model.to_ascii_lowercase();
                        [
                            format!("model {model}"),
                            format!("model '{model}'"),
                            format!("model \"{model}\""),
                            format!("model `{model}`"),
                        ]
                        .contains(&text)
                    });
                if exact_model
                    && fields
                        .get("type")
                        .and_then(Value::as_str)
                        .is_some_and(|kind| {
                            matches!(
                                kind.to_ascii_lowercase().as_str(),
                                "not_found" | "not_found_error"
                            )
                        })
                {
                    return true;
                }
                fields.iter().any(|(key, value)| {
                    let direct = value.as_str().is_some_and(|text| {
                        match key.to_ascii_lowercase().as_str() {
                            "code" | "type" => identifier(text),
                            "error" | "message" | "detail" | "error_description" | "title" => {
                                missing_message(text, model)
                            }
                            _ => false,
                        }
                    });
                    direct || (value.is_object() || value.is_array()) && structured(value, model)
                })
            }
            Value::String(text) => missing_message(text, model),
            _ => false,
        }
    }
    if structured(value, model) {
        return true;
    }
    if !matches!(status, 400 | 404 | 422) {
        return false;
    }
    if [
        "/error/code",
        "/error/type",
        "/response/error/code",
        "/response/error/type",
        "/code",
        "/type",
    ]
    .iter()
    .any(|p| {
        value
            .pointer(p)
            .and_then(Value::as_str)
            .is_some_and(|id| id.eq_ignore_ascii_case("model_not_supported"))
    }) {
        return true;
    }
    let message = upstream_error_metadata(value)
        .0
        .unwrap_or_default()
        .to_ascii_lowercase();
    [
        "model_not_supported",
        "requested model is not supported",
        "requested model is unsupported",
        "requested model is unavailable",
        "model is not supported",
        "model not supported",
        "unsupported model",
        "model unavailable",
        "not available for your plan",
        "not available for your account",
    ]
    .iter()
    .any(|part| message.contains(part))
}

pub(crate) fn model_unavailable_message(status: u16, message: &str, model: &str) -> bool {
    let value =
        serde_json::from_str::<Value>(message).unwrap_or_else(|_| Value::String(message.into()));
    model_unavailable(status, &value, model)
}

pub(super) fn chat_failure(value: &Value, fallback: u16) -> Option<ServiceError> {
    let kind = value
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if !["error", "response.error", "response.failed"]
        .iter()
        .any(|v| kind.eq_ignore_ascii_case(v))
        && !["/error", "/response/error"]
            .iter()
            .any(|p| value.pointer(p).is_some_and(|v| !v.is_null()))
        && !(value.get("code").is_some() && value.get("message").is_some())
    {
        return None;
    }
    let status = [
        "/status",
        "/status_code",
        "/error/status",
        "/error/status_code",
        "/response/error/status",
        "/response/error/status_code",
    ]
    .iter()
    .find_map(|p| {
        value
            .pointer(p)
            .and_then(|v| v.as_u64().or_else(|| v.as_str()?.parse().ok()))
            .and_then(|s| u16::try_from(s).ok())
            .filter(|s| (400..=599).contains(s))
    })
    .unwrap_or_else(|| {
        // Retain the existing known-code compatibility when no explicit
        // upstream status exists; additional status fields always win.
        let code = super::diagnostics::error_object(value)
            .get("code")
            .and_then(Value::as_str)
            .unwrap_or("");
        match code {
            "usage_limit_reached" | "insufficient_quota" | "rate_limit_exceeded" => 429,
            "invalid_api_key" | "invalid_token" | "invalid_grant" | "reauthorization_required" => {
                401
            }
            "account_deactivated" | "account_suspended" => 403,
            code if super::chatgpt_plan_wire::error_status(code).is_some() =>
                super::chatgpt_plan_wire::error_status(code).unwrap(),
            _ => {
                // Opening probes use zero to distinguish an absent upstream
                // status from the normal reporting fallback of 502.
                if fallback == 0 || (400..=599).contains(&fallback) {
                    fallback
                } else {
                    502
                }
            }
        }
    });
    let (message, scope) = super::diagnostics::upstream_error_metadata_status(status, value);
    let extracted = super::diagnostics::error_object(value);
    let details = (!std::ptr::eq(extracted, value) && extracted.is_object())
        .then(|| extracted.clone())
        .unwrap_or_else(|| {
            let mut details = json!({});
            for key in [
                "code",
                "message",
                "param",
                "retryable",
                "details",
                "status",
                "status_code",
                "request_id",
            ] {
                if let Some(field) = value.get(key) {
                    details[key] = field.clone();
                }
            }
            if let Some(kind) = value.get("error_type").and_then(Value::as_str) {
                details["type"] = json!(kind);
            }
            details
        });
    Some(
        ServiceError::new(
            status,
            if request_fault(status, value) {
                REQUEST_SCOPED
            } else if model_unavailable(status, value, "") {
                "upstream_model_unavailable"
            } else {
                "upstream_error"
            },
            message.unwrap_or_else(|| "Chat upstream returned an error".into()),
        )
        .with_credential_scope(scope)
        .with_error_details(details),
    )
}

pub(super) fn chat_frame_failure(frame: &SseFrame) -> Option<ServiceError> {
    if let Some(value) = &frame.value {
        return chat_failure(value, 502);
    }
    let text = String::from_utf8_lossy(&frame.bytes);
    text.lines()
        .filter_map(|line| line.trim().strip_prefix("event:"))
        .find_map(|event| chat_failure(&json!({"type":event.trim()}), 502))
}

pub(super) fn canonical_chat_failure(frame: &SseFrame, error: &ServiceError) -> Value {
    let mut value = frame
        .value
        .clone()
        .filter(Value::is_object)
        .unwrap_or_else(|| json!({}));
    if value.get("error").is_none_or(Value::is_null) {
        value["error"] = value
            .pointer("/response/error")
            .filter(|v| !v.is_null())
            .cloned()
            .unwrap_or_else(|| {
                let mut details = flat_error_details(&value);
                let fields = details.as_object_mut().unwrap();
                if fields.is_empty() {
                    fields.insert("code".into(), json!(error.code));
                }
                fields
                    .entry("message")
                    .or_insert_with(|| json!(error.message));
                details
            });
    }
    value["type"] = json!("error");
    value["status"] = json!(error.status);
    value
}

/// Preserve the original flat failure fields before turning its `type` into an
/// event discriminator. Existing nested provider envelopes are never replaced.
pub(super) fn flat_error_details(value: &Value) -> Value {
    let mut fields = serde_json::Map::new();
    for key in [
        "code",
        "type",
        "message",
        "status",
        "status_code",
        "param",
        "error_description",
    ] {
        if let Some(value) = value.get(key) {
            fields.insert(key.into(), value.clone());
        }
    }
    if let Some(kind) = value.get("original_error_type") {
        fields.insert("type".into(), kind.clone());
    }
    if fields
        .get("type")
        .and_then(Value::as_str)
        .is_some_and(|kind| matches!(kind, "error" | "response.error" | "response.failed"))
    {
        fields.remove("type");
    }
    Value::Object(fields)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::modules::codex_native_api::events::SseDecoder;

    #[test]
    fn request_scope_and_auth_type_match_actual_go_soft_failure_fixture() {
        let cases: Vec<Value> =
            serde_json::from_str(include_str!("fixtures/soft_affinity_failures_go.json")).unwrap();
        for case in cases.into_iter().filter(|case| case.get("code").is_none()) {
            let raw = case["body"].as_str().unwrap();
            let body =
                serde_json::from_str::<Value>(raw).unwrap_or_else(|_| Value::String(raw.into()));
            let status = case["status"].as_u64().unwrap() as u16;
            assert_eq!(
                request_fault(status, &body),
                case["skip"].as_bool().unwrap(),
                "{}",
                case["name"]
            );
        }
        let body = json!({"error":{"code":"invalid_request_error","type":"authentication_error"}});
        assert!(super::super::diagnostics::upstream_error_metadata_status(401, &body).1);
        assert!(!super::super::diagnostics::upstream_error_metadata_status(400, &body).1);
        for status in [401, 403, 503] {
            let body = json!({"error":{"code":"invalid_request_error","message":"request"}});
            assert_eq!(
                chat_failure(&json!({"status":status,"error":body["error"]}), 502)
                    .unwrap()
                    .code,
                REQUEST_SCOPED
            );
            let event = json!({"type":"error","status":status,"error":{"code":"invalid_request_error","message":"model is at capacity"}});
            assert!(super::super::bootstrap::rejection(&event).is_none());
        }
    }

    #[test]
    fn model_availability_matches_codes_messages_and_request_fault_boundaries() {
        for (status, value) in [
            (
                400,
                json!({"error":{"code":"model_not_supported","message":"not enabled"}}),
            ),
            (
                404,
                json!({"error":{"message":"The requested model is unavailable"}}),
            ),
            (
                422,
                json!({"error":{"message":"not available for your account"}}),
            ),
            (
                502,
                json!({"error":{"type":"https://errors.invalid/#model-not-found"}}),
            ),
            (
                404,
                json!({"error":{"type":"not_found_error","message":"model 'gpt-fixture'"}}),
            ),
            (
                404,
                json!({"error":{"message":"Model 'gpt-fixture' does not exist"}}),
            ),
        ] {
            assert!(
                model_unavailable(status, &value, "gpt-fixture"),
                "{status}: {value}"
            );
        }
        for value in [
            json!({"error":{"message":"invalid parameter"}}),
            json!({"error":{"type":"not_found_error","message":"response 'r1'"}}),
            json!({"error":{"message":"model_not_found in request body is invalid"}}),
            json!({"error":{"code":"invalid_encrypted_content","message":"model not supported"}}),
            json!({"error":{"code":"previous_response_not_found","message":"requested model is unavailable"}}),
        ] {
            assert!(!model_unavailable(400, &value, "gpt-fixture"), "{value}");
        }
        assert!(!model_unavailable(
            500,
            &json!({"error":{"message":"unsupported model"}}),
            "gpt-fixture"
        ));
    }

    #[test]
    fn chat_error_envelopes_preserve_explicit_status_and_known_code_compatibility() {
        for (value, expected) in [
            (json!({"error":{"status":429,"message":"limited"}}), 429),
            (
                json!({"response":{"error":{"status_code":403,"message":"denied"}}}),
                403,
            ),
            (
                json!({"type":"response.error","status_code":503,"message":"overloaded"}),
                503,
            ),
            (
                json!({"code":"error","message":"failed","status":"504"}),
                504,
            ),
            (
                json!({"error":{"code":"rate_limit_exceeded","message":"limited"}}),
                429,
            ),
            (
                json!({"type":"error","status":200,"error":{"status_code":401}}),
                401,
            ),
        ] {
            assert_eq!(
                chat_failure(&value, 502).unwrap().status,
                expected,
                "{value}"
            );
        }
        assert!(
            chat_failure(&json!({"choices":[{"delta":{"content":"error 503"}}]}), 502).is_none()
        );
    }

    #[test]
    fn chat_declared_error_frames_without_error_object_are_failures() {
        for raw in [
            "event: error\n\n",
            "event: response.error\ndata: [DONE]\n\n",
            "event: response.failed\ndata: {\"message\":\"failed\",\"status\":503}\n\n",
        ] {
            let mut decoder = SseDecoder::new(4096);
            let frames = decoder.push_frames(raw.as_bytes()).unwrap();
            assert_eq!(frames.len(), 1);
            assert!(chat_frame_failure(&frames[0]).is_some());
        }
    }
}
