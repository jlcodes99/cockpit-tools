//! xAI returns invalid OAuth tokens as 403 and free quota as a rolling 24h 429.
//! These adjustments are applied only after selecting a Grok route.
use serde_json::{json, Value};
use std::time::Duration;

pub(super) fn applies(route: &super::contract::Route) -> bool {
    route.headers.contains_key("x-xai-token-auth")
        || (route.wire_api == super::contract::WireApi::Responses && official_api(&route.base_url))
}

fn official_api(base_url: &str) -> bool {
    reqwest::Url::parse(base_url)
        .is_ok_and(|url| url.scheme() == "https" && url.host_str() == Some("api.x.ai"))
}

fn text(value: &Value) -> String {
    value
        .as_str()
        .map(str::to_owned)
        .unwrap_or_else(|| value.to_string())
        .to_ascii_lowercase()
}
fn bad_credentials(value: &Value) -> bool {
    let text = text(value);
    text.contains("bad-credentials") || text.contains("access token could not be validated")
}
fn free_usage(value: &Value) -> bool {
    let text = text(value);
    text.contains("free-usage-exhausted") || text.contains("included free usage")
}

pub(super) fn classify(status: u16, value: &Value) -> (u16, Option<Duration>) {
    if status == 403 && bad_credentials(value) {
        (401, None)
    } else if status == 429 && free_usage(value) {
        (429, Some(Duration::from_secs(24 * 60 * 60)))
    } else {
        (status, None)
    }
}

pub(super) fn rejection(event: &Value) -> Option<(u16, Value)> {
    if !matches!(
        event.get("type").and_then(Value::as_str),
        Some("error" | "response.error" | "response.failed")
    ) || event
        .pointer("/response/output")
        .and_then(Value::as_array)
        .is_some_and(|output| !output.is_empty())
    {
        return None;
    }
    let explicit = super::upstream_errors::chat_failure(event, 0)
        .map(|error| error.status)
        .unwrap_or(0);
    if super::upstream_errors::request_fault(explicit, event) {
        return None;
    }
    let status = if matches!(explicit, 0 | 401 | 403) && bad_credentials(event) {
        401
    } else if matches!(explicit, 0 | 429) && free_usage(event) {
        429
    } else {
        return None;
    };
    let mut body = json!({"error":super::diagnostics::error_object(event)});
    if status == 429 {
        body["retry_after"] = json!(super::events::retry_after_event(event)
            .unwrap_or_default()
            .max(Duration::from_secs(24 * 60 * 60))
            .as_secs_f64());
    }
    Some((status, body))
}

pub(super) fn normalize_event(event: &mut Value) {
    if let Some((status, body)) = rejection(event) {
        event["status"] = json!(status);
        if let Some(wait) = body.get("retry_after") {
            event["retry_after"] = wait.clone();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn grok_official_api_match_does_not_capture_custom_providers() {
        assert!(official_api("https://api.x.ai/v1"));
        for url in [
            "http://api.x.ai/v1",
            "https://api.x.ai.example/v1",
            "https://other.example/api.x.ai",
            "https://user:pass@example.invalid/v1",
            "invalid",
        ] {
            assert!(!official_api(url), "{url}");
        }
    }

    #[test]
    fn grok_errors_only_refresh_invalid_tokens_and_cool_down_explicit_free_quota() {
        for value in [
            json!({"code":"bad-credentials","error":"invalid token"}),
            json!({"error":{"message":"Access token could not be validated"}}),
        ] {
            assert_eq!(classify(403, &value), (401, None));
            assert_eq!(classify(400, &value), (400, None));
        }
        assert_eq!(
            classify(403, &json!({"error":"permission denied"})),
            (403, None)
        );
        assert_eq!(
            classify(429, &json!({"error":"rate limit exceeded"})),
            (429, None)
        );
        assert_eq!(
            classify(429, &json!({"code":"subscription:free-usage-exhausted"})),
            (429, Some(Duration::from_secs(86400)))
        );
    }
    #[test]
    fn grok_error_events_keep_explicit_request_faults_and_longer_reset_hints() {
        for status in [400, 422, 500] {
            for code in ["bad-credentials", "subscription:free-usage-exhausted"] {
                let event = json!({"type":"error","status":status,"error":{"code":code}});
                assert!(rejection(&event).is_none(), "{event}");
            }
        }
        let fault = json!({"type":"error","error":{"type":"invalid_request_error","message":"included free usage"}});
        assert!(rejection(&fault).is_none());
        let mut event = json!({"type":"error","status":429,"retry_after":100000,"error":{"code":"subscription:free-usage-exhausted"}});
        normalize_event(&mut event);
        assert_eq!(event["retry_after"].as_f64(), Some(100000.0));
    }

    #[test]
    fn grok_error_events_preserve_the_cause_without_replaying_partial_output() {
        let mut event = json!({"type":"response.failed","response":{"output":[],"error":{"code":"bad-credentials","message":"original reason"}}});
        normalize_event(&mut event);
        assert_eq!(event["status"], 401);
        assert_eq!(event["response"]["error"]["message"], "original reason");
        event["response"]["output"] = json!([{"type":"message"}]);
        assert!(rejection(&event).is_none());
        let mut free = json!({"type":"error","error":{"code":"subscription:free-usage-exhausted"}});
        normalize_event(&mut free);
        assert_eq!(free["status"], 429);
        assert_eq!(free["retry_after"].as_f64(), Some(86400.0));
    }
}
