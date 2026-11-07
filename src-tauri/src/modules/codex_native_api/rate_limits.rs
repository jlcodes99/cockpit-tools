//! Quota snapshots carried by Codex HTTP responses and WebSocket/SSE events.
//! Token usage is deliberately not converted into an account allowance.
use axum::http::HeaderMap;
use serde_json::Value;

#[derive(Clone, Debug, PartialEq)]
pub struct QuotaWindow {
    pub remaining_percent: f64,
    pub window_minutes: Option<i64>,
    pub resets_at: Option<i64>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct QuotaSnapshot {
    pub primary: Option<QuotaWindow>,
    pub secondary: Option<QuotaWindow>,
}

fn window(used: f64, minutes: Option<i64>, reset: Option<i64>) -> Option<QuotaWindow> {
    (used.is_finite() && (0.0..=100.0).contains(&used)).then_some(QuotaWindow {
        remaining_percent: 100.0 - used,
        window_minutes: minutes.filter(|value| *value > 0),
        resets_at: reset.filter(|value| *value > 0),
    })
}

impl QuotaSnapshot {
    pub fn is_empty(&self) -> bool {
        self.primary.is_none() && self.secondary.is_none()
    }

    pub fn from_headers(headers: &HeaderMap) -> Option<Self> {
        let read = |name: &str| headers.get(name)?.to_str().ok();
        let parse = |prefix: &str| {
            window(
                read(&format!("x-codex-{prefix}-used-percent"))?
                    .parse()
                    .ok()?,
                read(&format!("x-codex-{prefix}-window-minutes")).and_then(|v| v.parse().ok()),
                read(&format!("x-codex-{prefix}-reset-at")).and_then(|v| v.parse().ok()),
            )
        };
        let snapshot = Self {
            primary: parse("primary"),
            secondary: parse("secondary"),
        };
        (!snapshot.is_empty()).then_some(snapshot)
    }

    pub fn from_event(event: &Value) -> Option<Self> {
        if event["type"] != "codex.rate_limits" {
            return None;
        }
        // Model-specific allowances must not replace the ordinary Codex pool.
        if let Some(id) = event
            .get("metered_limit_name")
            .filter(|id| !id.is_null())
            .or_else(|| event.get("limit_name").filter(|id| !id.is_null()))
        {
            if !id
                .as_str()
                .is_some_and(|id| id.trim().eq_ignore_ascii_case("codex"))
            {
                return None;
            }
        }
        let parse = |name: &str| {
            let value = event.get("rate_limits")?.get(name)?;
            window(
                value.get("used_percent")?.as_f64()?,
                value.get("window_minutes").and_then(Value::as_i64),
                value.get("reset_at").and_then(Value::as_i64),
            )
        };
        let snapshot = Self {
            primary: parse("primary"),
            secondary: parse("secondary"),
        };
        (!snapshot.is_empty()).then_some(snapshot)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn headers_keep_fractional_quota_and_missing_windows() {
        let mut headers = HeaderMap::new();
        headers.insert("x-codex-primary-used-percent", "89.75".parse().unwrap());
        headers.insert("x-codex-primary-window-minutes", "300".parse().unwrap());
        let snapshot = QuotaSnapshot::from_headers(&headers).unwrap();
        assert_eq!(snapshot.primary.unwrap().remaining_percent, 10.25);
        assert!(snapshot.secondary.is_none());
        headers.insert("x-codex-primary-used-percent", "NaN".parse().unwrap());
        assert!(QuotaSnapshot::from_headers(&headers).is_none());
        headers.insert("x-codex-primary-used-percent", "101".parse().unwrap());
        assert!(QuotaSnapshot::from_headers(&headers).is_none());
    }

    #[test]
    fn events_do_not_confuse_tokens_or_other_model_limits_with_quota() {
        let mut event = json!({"type":"codex.rate_limits", "rate_limits":{
            "primary":{"used_percent":90.5,"window_minutes":300,"reset_at":123456}}});
        assert_eq!(
            QuotaSnapshot::from_event(&event)
                .unwrap()
                .primary
                .unwrap()
                .remaining_percent,
            9.5
        );
        event["metered_limit_name"] = Value::Null;
        event["limit_name"] = Value::Null;
        assert!(QuotaSnapshot::from_event(&event).is_some());
        event["metered_limit_name"] = json!("gpt-reserve");
        assert!(QuotaSnapshot::from_event(&event).is_none());
        assert!(
            QuotaSnapshot::from_event(&json!({"type":"response.completed",
            "response":{"usage":{"total_tokens":100}}}))
            .is_none()
        );
        assert!(
            QuotaSnapshot::from_event(&json!({"type":"codex.rate_limits",
            "rate_limits":{"primary":{"used_percent":-1}}}))
            .is_none()
        );
    }
}
