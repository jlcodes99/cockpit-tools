use super::{
    json_f64_field, json_timestamp_field, push_usage_detail, CodexModelProviderUsageSummary,
    CODEX_MODEL_PROVIDER_TEST_TIMEOUT_SECS,
};
use serde_json::Value;
use std::time::{Duration, Instant};

const OPENCODE_GO_USAGE_URL: &str = "https://opencode.ai/zen/go/v1/usage";

pub(super) fn is_opencode_go_endpoint(base_url: &str) -> bool {
    let Ok(url) = reqwest::Url::parse(base_url.trim()) else {
        return false;
    };
    url.scheme() == "https"
        && url.host_str() == Some("opencode.ai")
        && url.port().is_none()
        && url.username().is_empty()
        && url.password().is_none()
        && matches!(
            url.path().trim_end_matches('/'),
            "/zen/go" | "/zen/go/v1" | "/inference/go/openai/v1" | "/inference/go/v1"
        )
}

pub(super) async fn query_opencode_go(key: &str) -> Result<CodexModelProviderUsageSummary, String> {
    let started = Instant::now();
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(CODEX_MODEL_PROVIDER_TEST_TIMEOUT_SECS))
        .build()
        .map_err(|_| "CREATE_HTTP_CLIENT_FAILED")?;
    let response = client
        .get(OPENCODE_GO_USAGE_URL)
        .bearer_auth(key)
        .send()
        .await
        .map_err(|_| "PROVIDER_USAGE_NETWORK_FAILED")?;
    if !response.status().is_success() {
        return Err(format!(
            "PROVIDER_USAGE_HTTP_{}",
            response.status().as_u16()
        ));
    }
    let body = response
        .json()
        .await
        .map_err(|_| "PROVIDER_USAGE_PARSE_FAILED: OpenCode Go")?;
    summarize_opencode_go(&body, started.elapsed().as_millis() as u64)
}

fn summarize_opencode_go(
    body: &Value,
    latency_ms: u64,
) -> Result<CodexModelProviderUsageSummary, String> {
    let mut min_remaining: f64 = 100.0;
    let mut details = Vec::new();
    for (key, remaining_key, reset_key, label) in [
        (
            "rolling",
            "intervalRemainingPercent",
            "intervalExpiresAt",
            "5-hour",
        ),
        (
            "weekly",
            "weeklyRemainingPercent",
            "weeklyExpiresAt",
            "Weekly",
        ),
        (
            "monthly",
            "monthlyRemainingPercent",
            "monthlyExpiresAt",
            "Monthly",
        ),
    ] {
        let legacy_key = format!("{key}Usage");
        let window = body["usage"]
            .get(key)
            .or_else(|| body.get(&legacy_key))
            .ok_or("PROVIDER_USAGE_PARSE_FAILED: OpenCode Go window missing")?;
        let remaining = match window["status"].as_str() {
            Some("rate-limited") => 0.0,
            Some("ok") => {
                100.0
                    - json_f64_field(window, &["percent"])
                        .filter(|amount| (0.0..=100.0).contains(amount))
                        .ok_or("PROVIDER_USAGE_PARSE_FAILED: OpenCode Go percent")?
            }
            _ => return Err("PROVIDER_USAGE_PARSE_FAILED: OpenCode Go window status".into()),
        };
        let reset = json_timestamp_field(window, &["resetsAt", "resetAt"]);
        min_remaining = min_remaining.min(remaining);
        push_usage_detail(
            &mut details,
            remaining_key,
            label,
            Some(remaining.to_string()),
        );
        push_usage_detail(
            &mut details,
            reset_key,
            label,
            reset.map(|time| time.to_string()),
        );
    }
    Ok(CodexModelProviderUsageSummary {
        mode: Some("token_plan".into()),
        is_valid: Some(true),
        plan_name: Some("OpenCode Go".into()),
        unit: Some("%".into()),
        quota_limit: Some(100.0),
        quota_used: Some(100.0 - min_remaining),
        quota_remaining: Some(min_remaining),
        latency_ms,
        details,
        ..Default::default()
    })
}

pub(super) fn is_codex_proxy_rs_usage(body: &Value) -> bool {
    body["unit"] == "USD"
        && ["daily", "weekly"].into_iter().all(|key| {
            ["total", "used", "remaining", "resetsAt"]
                .into_iter()
                .all(|field| body[key].get(field).is_some())
        })
}

pub(super) fn summarize_codex_proxy_rs(
    body: &Value,
    latency_ms: u64,
) -> Result<CodexModelProviderUsageSummary, String> {
    let mut details = Vec::new();
    let mut tightest_window: Option<(f64, f64, f64)> = None;
    for (window_key, limit_key, remaining_key, reset_key) in [
        ("daily", "dailyLimit", "dailyRemaining", "dailyExpiresAt"),
        (
            "weekly",
            "weeklyLimit",
            "weeklyRemaining",
            "weeklyExpiresAt",
        ),
    ] {
        let window = &body[window_key];
        let limit = json_f64_field(window, &["total"]);
        let remaining = json_f64_field(window, &["remaining"]);
        if !window["total"].is_null() {
            let amounts = limit
                .zip(json_f64_field(window, &["used"]))
                .zip(remaining)
                .map(|((limit, used), remaining)| (limit, used, remaining))
                .filter(|(limit, used, remaining)| {
                    [limit, used, remaining]
                        .into_iter()
                        .all(|value| value.is_finite() && *value >= 0.0)
                })
                .ok_or("PROVIDER_USAGE_PARSE_FAILED: codex-proxy-rs quota")?;
            if tightest_window.is_none_or(|(_, _, available)| amounts.2 < available) {
                tightest_window = Some(amounts);
            }
        }
        for (field, key) in [("total", limit_key), ("remaining", remaining_key)] {
            push_usage_detail(
                &mut details,
                key,
                key,
                window[field].as_str().map(str::to_owned),
            );
        }
        let reset = json_timestamp_field(window, &["resetsAt"]);
        push_usage_detail(
            &mut details,
            reset_key,
            reset_key,
            reset.map(|time| time.to_string()),
        );
    }
    Ok(CodexModelProviderUsageSummary {
        mode: Some("token_plan".into()),
        plan_name: Some("codex-proxy-rs".into()),
        is_valid: Some(true),
        unit: Some("USD".into()),
        quota_unlimited: Some(tightest_window.is_none()),
        quota_limit: tightest_window.map(|amounts| amounts.0),
        quota_used: tightest_window.map(|amounts| amounts.1),
        quota_remaining: tightest_window.map(|amounts| amounts.2),
        latency_ms,
        details,
        ..Default::default()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn opencode_go_uses_only_official_endpoints_and_counts_rate_limited_windows() {
        assert!(is_opencode_go_endpoint(
            "https://opencode.ai/inference/go/openai/v1/"
        ));
        assert!(!is_opencode_go_endpoint(
            "https://opencode.ai.example/zen/go/v1"
        ));
        assert!(!is_opencode_go_endpoint(
            "https://opencode.ai:1234/zen/go/v1"
        ));
        let body = json!({"usage": {
            "rolling": {"status": "ok", "percent": 45, "resetsAt": "2026-11-14T16:00:00Z"},
            "weekly": {"status": "rate-limited", "percent": 99},
            "monthly": {"status": "ok", "percent": 75}
        }});
        let result = summarize_opencode_go(&body, 8).unwrap();
        assert_eq!(result.quota_remaining, Some(0.0));
        assert!(result
            .details
            .iter()
            .any(|item| item.key == "monthlyRemainingPercent" && item.value == "25"));
        assert!(result
            .details
            .iter()
            .any(|item| item.key == "intervalExpiresAt"));
        assert!(summarize_opencode_go(&json!({}), 0).is_err());
    }

    #[test]
    fn codex_proxy_rs_uses_the_tightest_finite_window() {
        let body = json!({"unit":"USD",
            "daily":{"total":null,"used":"24.123","remaining":null,"resetsAt":null},
            "weekly":{"total":"250","used":"3.09679626","remaining":"246.90320374","resetsAt":"2026-11-10T16:00:00Z"}
        });
        assert!(is_codex_proxy_rs_usage(&body));
        let result = summarize_codex_proxy_rs(&body, 1).unwrap();
        assert_eq!(result.quota_remaining, Some(246.90320374));
        assert_eq!(result.quota_unlimited, Some(false));
        assert!(result
            .details
            .iter()
            .any(|item| item.key == "weeklyRemaining" && item.value == "246.90320374"));
        assert!(!is_codex_proxy_rs_usage(
            &json!({"unit":"USD","remaining":9})
        ));
    }
}
