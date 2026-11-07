//! Codex admission throttling is model capacity, never exhausted credential quota.
use serde_json::Value;

pub(crate) fn applies(value: &Value) -> bool {
    if [
        "/error/type",
        "/response/error/type",
        "/body/error/type",
        "/type",
    ]
    .iter()
    .filter_map(|path| value.pointer(path).and_then(Value::as_str))
    .any(|kind| {
        matches!(
            kind.to_ascii_lowercase().as_str(),
            "usage_limit_reached" | "insufficient_quota" | "quota_exceeded"
        )
    }) {
        return false;
    }
    let code = [
        "/error/code",
        "/response/error/code",
        "/body/error/code",
        "/code",
    ]
    .iter()
    .filter_map(|path| value.pointer(path).and_then(Value::as_str))
    .find(|code| !code.trim().is_empty());
    if let Some(code) = code {
        return matches!(
            code.trim().to_ascii_lowercase().as_str(),
            "turn_rate_limit_exceeded" | "model_at_capacity" | "model_is_at_capacity"
        );
    }
    let error = super::diagnostics::error_object(value);
    error["type"]
        .as_str()
        .is_some_and(|kind| kind.eq_ignore_ascii_case("rate_limit_error"))
        && error["message"]
            .as_str()
            .is_some_and(|message| message.to_ascii_lowercase().contains("at capacity"))
}
