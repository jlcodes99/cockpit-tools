//! Request-only SIWC compatibility. Never rewrite the replay/history source.
use axum::http::{header, HeaderMap, HeaderValue};
use serde_json::{json, Value};

use super::contract::ServiceError;

fn unsupported_configuration(index: usize) -> ServiceError {
    let param = format!("input[{index}]");
    let message = "ChatGPT plan configuration updates can only change reasoning effort";
    ServiceError::new(400, "subscription_sharing_unsupported_capability", message)
        .with_error_details(json!({
            "code":"subscription_sharing_unsupported_capability",
            "message":message,
            "param":param,
        }))
}

/// A configuration update must not silently discard a tool declaration,
/// tool_choice restriction, or an unknown future setting. The public Responses
/// update contract currently changes only reasoning effort.
pub(super) fn validate_configuration_updates(body: &Value) -> Result<(), ServiceError> {
    let Some(input) = body.get("input").and_then(Value::as_array) else {
        return Ok(());
    };
    for (index, item) in input.iter().enumerate() {
        if item["type"] != "configuration_update" {
            continue;
        }
        let object = item.as_object().expect("typed update is an object");
        if object
            .keys()
            .any(|key| !matches!(key.as_str(), "type" | "id" | "reasoning"))
        {
            return Err(unsupported_configuration(index));
        }
        if let Some(reasoning) = object.get("reasoning").filter(|value| !value.is_null()) {
            if reasoning
                .as_object()
                .is_none_or(|value| value.keys().any(|key| key != "effort"))
                || reasoning
                    .get("effort")
                    .is_some_and(|effort| !effort.is_null() && !effort.is_string())
            {
                return Err(unsupported_configuration(index));
            }
        }
    }
    Ok(())
}

/// SIWC currently rejects configuration_update input items. Collapse their
/// last usable effort into the request-level setting, accepting prompt-cache
/// prefix changes. An already resolved explicit model suffix takes precedence.
/// Validate every item before mutation so a rejected request stays unchanged.
pub(super) fn collapse_configuration_updates(
    body: &mut Value,
    keep_top_level_effort: bool,
) -> Result<(), ServiceError> {
    validate_configuration_updates(body)?;
    let Some(input) = body.get("input").and_then(Value::as_array) else {
        return Ok(());
    };
    let mut has_update = false;
    let mut effort = None;
    for item in input {
        if item["type"] == "configuration_update" {
            has_update = true;
            if let Some(value) = item["reasoning"]["effort"].as_str() {
                let value = value.trim();
                if !value.is_empty() {
                    effort = Some(value.to_ascii_lowercase());
                }
            }
        }
    }
    if !has_update {
        return Ok(());
    }
    if !keep_top_level_effort
        && effort.is_some()
        && body
            .get("reasoning")
            .is_some_and(|value| !value.is_null() && !value.is_object())
    {
        return Err(
            ServiceError::new(400, "invalid_request_error", "reasoning must be an object")
                .with_error_details(json!({
                    "code":"invalid_request_error",
                    "message":"reasoning must be an object",
                    "param":"reasoning",
                })),
        );
    }
    body["input"]
        .as_array_mut()
        .expect("array checked above")
        .retain(|item| item["type"] != "configuration_update");
    if !keep_top_level_effort {
        if let Some(effort) = effort {
            if body.get("reasoning").is_none_or(Value::is_null) {
                body["reasoning"] = json!({});
            }
            body["reasoning"]["effort"] = Value::String(effort);
        }
    }
    Ok(())
}

/// Public SIWC requests identify this application and must not inherit Codex
/// backend feature/routing headers from the downstream client. Correlation IDs
/// have already passed through the caller/account identity boundary; retain them.
pub(super) fn prepare_headers(headers: &mut HeaderMap) {
    let internal: Vec<_> = headers
        .keys()
        .filter(|name| {
            matches!(
                name.as_str(),
                "originator" | "openai-beta" | "chatgpt-account-id"
            ) || name.as_str().starts_with("x-codex-")
                || name.as_str().starts_with("x-openai-internal-")
        })
        .cloned()
        .collect();
    for name in internal {
        headers.remove(name);
    }
    headers.insert(
        header::USER_AGENT,
        HeaderValue::from_static(concat!("CockpitTools/", env!("CARGO_PKG_VERSION"))),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn last_valid_effort_wins_and_other_request_settings_survive() {
        let retained = json!([
            {"role":"user","content":"configuration_update is plain text"},
            {"type":"reasoning","encrypted_content":"unchanged"},
            {"type":"function_call","call_id":"c","name":"read","arguments":"{}"},
            {"type":"function_call_output","call_id":"c","output":"unchanged"},
        ]);
        let mut input = retained.as_array().unwrap().clone();
        input.insert(
            0,
            json!({"type":"configuration_update","reasoning":{"effort":"low"}}),
        );
        input.push(json!({"type":"configuration_update","reasoning":{"effort":" High "}}));
        for unusable in [Value::Null, json!("  ")] {
            input.push(json!({"type":"configuration_update","reasoning":{"effort":unusable}}));
        }
        let mut body = json!({
            "input":input,
            "reasoning":{"effort":"medium","summary":"auto","other":7},
            "tools":[],"tool_choice":"none","parallel_tool_calls":false,
            "prompt_cache_key":"already-isolated",
        });
        collapse_configuration_updates(&mut body, false).unwrap();
        assert_eq!(body["input"], retained);
        assert_eq!(
            body["reasoning"],
            json!({"effort":"high","summary":"auto","other":7})
        );
        assert_eq!(body["tools"], json!([]));
        assert_eq!(body["tool_choice"], "none");
        assert_eq!(body["parallel_tool_calls"], false);
        assert_eq!(body["prompt_cache_key"], "already-isolated");
    }

    #[test]
    fn explicit_suffix_effort_is_not_overridden_by_history() {
        let mut body = json!({"reasoning":{"effort":"low","summary":"auto"},"input":[{"type":"configuration_update","reasoning":{"effort":"high"}}]});
        collapse_configuration_updates(&mut body, true).unwrap();
        assert_eq!(
            body,
            json!({"reasoning":{"effort":"low","summary":"auto"},"input":[]})
        );
    }

    #[test]
    fn missing_reasoning_is_created_without_inventing_effort_levels() {
        for effort in ["none", "auto", "xhigh", "future-level"] {
            let mut body =
                json!({"input":[{"type":"configuration_update","reasoning":{"effort":effort}}]});
            collapse_configuration_updates(&mut body, false).unwrap();
            assert_eq!(body, json!({"input":[],"reasoning":{"effort":effort}}));
        }
    }

    #[test]
    fn no_effort_updates_do_not_clear_the_existing_setting() {
        let mut body = json!({"reasoning":{"effort":"medium"},"input":[{"type":"configuration_update"},{"type":"configuration_update","reasoning":{}},{"type":"configuration_update","reasoning":null}]});
        collapse_configuration_updates(&mut body, false).unwrap();
        assert_eq!(body, json!({"reasoning":{"effort":"medium"},"input":[]}));
    }

    #[test]
    fn unrelated_payloads_are_not_modified_or_scanned_recursively() {
        for mut body in [
            json!({}),
            json!({"input":"configuration_update"}),
            json!({"input":[]}),
            json!({"input":{"type":"configuration_update","reasoning":{"effort":"high"}}}),
            json!({"input":[{"role":"user","content":[{"type":"configuration_update","reasoning":{"effort":"high"}}]}]}),
        ] {
            let original = body.clone();
            collapse_configuration_updates(&mut body, false).unwrap();
            assert_eq!(body, original);
        }
    }

    #[test]
    fn unknown_or_tool_configuration_is_rejected_before_any_mutation() {
        for setting in [
            json!({"tools":[]}),
            json!({"tools":[{"type":"code_interpreter"}]}),
            json!({"tool_choice":"none"}),
            json!({"parallel_tool_calls":false}),
            json!({"future_setting":true}),
            json!({"reasoning":{"effort":"high","summary":"auto"}}),
            json!({"reasoning":false}),
            json!({"reasoning":{"effort":42}}),
            json!({"reasoning":{"effort":false}}),
            json!({"reasoning":{"effort":[]}}),
            json!({"reasoning":{"effort":{}}}),
        ] {
            let mut update = json!({"type":"configuration_update"});
            update
                .as_object_mut()
                .unwrap()
                .extend(setting.as_object().unwrap().clone());
            let mut body = json!({"reasoning":{"effort":"low"},"input":[{"type":"configuration_update","reasoning":{"effort":"high"}},update]});
            let original = body.clone();
            let error = collapse_configuration_updates(&mut body, false).unwrap_err();
            assert_eq!(error.code, "subscription_sharing_unsupported_capability");
            assert_eq!(error.response_error.unwrap()["param"], "input[1]");
            assert_eq!(body, original);
        }
    }

    #[test]
    fn malformed_request_reasoning_is_not_silently_replaced() {
        let mut body = json!({"reasoning":"invalid","input":[{"type":"configuration_update","reasoning":{"effort":"high"}}]});
        let original = body.clone();
        let error = collapse_configuration_updates(&mut body, false).unwrap_err();
        assert_eq!(error.response_error.unwrap()["param"], "reasoning");
        assert_eq!(body, original);
    }

    #[test]
    fn public_headers_use_application_identity_and_keep_isolated_correlation() {
        let mut headers = HeaderMap::new();
        for name in [
            "originator",
            "openai-beta",
            "chatgpt-account-id",
            "x-codex-routing-hint",
            "x-codex-turn-state",
            "x-codex-window-id",
            "x-openai-internal-codex-responses-lite",
        ] {
            headers.insert(name, HeaderValue::from_static("client-backend-value"));
        }
        headers.insert(
            header::USER_AGENT,
            HeaderValue::from_static("codex-tui/client"),
        );
        for name in [
            "session-id",
            "thread-id",
            "x-client-request-id",
            "authorization",
        ] {
            headers.insert(name, HeaderValue::from_static("already-isolated"));
        }
        prepare_headers(&mut headers);
        assert_eq!(
            headers[header::USER_AGENT],
            concat!("CockpitTools/", env!("CARGO_PKG_VERSION"))
        );
        for name in [
            "session-id",
            "thread-id",
            "x-client-request-id",
            "authorization",
        ] {
            assert_eq!(headers[name], "already-isolated");
        }
        assert!(!headers
            .keys()
            .any(|name| name.as_str().starts_with("x-codex-")
                || name.as_str().starts_with("x-openai-internal-")));
        assert!(!headers.contains_key("originator"));
        assert!(!headers.contains_key("openai-beta"));
        assert!(!headers.contains_key("chatgpt-account-id"));
    }
}
