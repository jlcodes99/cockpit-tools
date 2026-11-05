//! codex-proxy-rs exposes daily/weekly budgets at /v1/usage.
use super::{CodexModelProviderUsageDetail, CodexModelProviderUsageSummary};
use serde_json::Value;
use std::time::{Duration, Instant};

pub fn matches(body: &Value) -> bool {
    body.get("unit").is_some() && body["daily"].is_object() && body["weekly"].is_object()
}

pub fn summarize(body: &Value, latency_ms: u64) -> Result<CodexModelProviderUsageSummary, String> {
    let invalid = "PROVIDER_USAGE_CODEX_PROXY_RS_PARSE_FAILED";
    let unit = body["unit"]
        .as_str()
        .filter(|s| !s.trim().is_empty() && s.len() <= 32)
        .ok_or(invalid)?;
    let mut details = Vec::new();
    for window in ["daily", "weekly"] {
        let data = body.get(window).filter(|v| v.is_object()).ok_or(invalid)?;
        for field in ["total", "used", "remaining"] {
            let value = match data.get(field).ok_or(invalid)? {
                // Null limits and remaining quotas mean unlimited; null usage stays unknown.
                Value::Null if field == "total" || field == "remaining" => "∞".to_owned(),
                Value::Null => "-".to_owned(),
                Value::Number(n) => n.to_string(),
                Value::String(s) if s.trim().parse::<f64>().ok().is_some_and(f64::is_finite) => {
                    s.trim().to_owned()
                }
                _ => return Err(invalid.into()),
            };
            // Preserve decimal strings in the shared details cache, without rounding persisted amounts.
            details.push(CodexModelProviderUsageDetail {
                key: format!("{window}.{field}"),
                label: format!("{window} {field}"),
                value,
            });
        }
        let reset = match data.get("resetsAt") {
            None | Some(Value::Null) => "-".to_owned(),
            Some(Value::String(s)) => chrono::DateTime::parse_from_rfc3339(s)
                .map_err(|_| invalid)?
                .to_rfc3339(),
            _ => return Err(invalid.into()),
        };
        details.push(CodexModelProviderUsageDetail {
            key: format!("{window}.resetsAt"),
            label: format!("{window} reset"),
            value: reset,
        });
    }
    Ok(CodexModelProviderUsageSummary {
        mode: Some("codex_proxy_rs".into()),
        unit: Some(unit.trim().into()),
        // Both windows must be unlimited before treating the whole account as unlimited.
        quota_unlimited: Some(body["daily"]["total"].is_null() && body["weekly"]["total"].is_null()),
        details,
        latency_ms,
        ..Default::default()
    })
}

fn usage_url(base_url: &str) -> Result<reqwest::Url, String> {
    let mut url = reqwest::Url::parse(base_url.trim()).map_err(|_| "PROVIDER_BASE_URL_INVALID")?;
    if !matches!(url.scheme(), "http" | "https")
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err("PROVIDER_BASE_URL_INVALID".into());
    }
    // Root URLs need /v1; already-versioned Base URLs must not become /v1/v1/usage.
    let path = url.path().trim_end_matches('/');
    let path = if path.is_empty() {
        "/v1/usage".to_owned()
    } else {
        format!("{path}/usage")
    };
    url.set_path(&path);
    url.set_query(None);
    url.set_fragment(None);
    Ok(url)
}

pub async fn query(base_url: &str, key: &str) -> Result<CodexModelProviderUsageSummary, String> {
    let url = usage_url(base_url)?;
    let started = Instant::now();
    // A balance query must not forward credentials to a redirect target or return echoed secrets.
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(20))
        .build()
        .map_err(|_| "PROVIDER_USAGE_NETWORK_FAILED")?;
    let mut response = client
        .get(url)
        .bearer_auth(key)
        .header("Accept", "application/json")
        .send()
        .await
        .map_err(|_| "PROVIDER_USAGE_NETWORK_FAILED")?;
    if !response.status().is_success() {
        return Err(format!(
            "PROVIDER_USAGE_HTTP_{}",
            response.status().as_u16()
        ));
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| "PROVIDER_USAGE_NETWORK_FAILED")?
    {
        if bytes.len() + chunk.len() > 1024 * 1024 {
            return Err("PROVIDER_USAGE_RESPONSE_TOO_LARGE".into());
        }
        bytes.extend_from_slice(&chunk);
    }
    let body =
        serde_json::from_slice(&bytes).map_err(|_| "PROVIDER_USAGE_CODEX_PROXY_RS_PARSE_FAILED")?;
    summarize(
        &body,
        started.elapsed().as_millis().try_into().unwrap_or(u64::MAX),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn daily_weekly_usage_preserves_precision_nulls_and_reset_time() {
        let mut body = serde_json::json!({"unit":"USD","daily":{"total":"250","used":"0","remaining":"250","resetsAt":null},"weekly":{"total":"250","used":"3.09679626","remaining":"246.90320374","resetsAt":"2026-11-10T16:00:00Z"}});
        assert!(matches(&body));
        let result = summarize(&body, 1).unwrap();
        assert_eq!(result.mode.as_deref(), Some("codex_proxy_rs"));
        assert_eq!(
            result
                .details
                .iter()
                .find(|d| d.key == "weekly.remaining")
                .unwrap()
                .value,
            "246.90320374"
        );
        assert_eq!(
            result
                .details
                .iter()
                .find(|d| d.key == "daily.resetsAt")
                .unwrap()
                .value,
            "-"
        );
        body["daily"]["remaining"] = Value::Null;
        assert_eq!(summarize(&body, 0).unwrap().details.iter().find(|d| d.key == "daily.remaining").unwrap().value, "∞");
        assert_eq!(summarize(&body, 0).unwrap().quota_unlimited, Some(false));
        body["daily"]["total"] = Value::Null;
        let unlimited_daily = summarize(&body, 0).unwrap();
        assert_eq!(unlimited_daily.details.iter().find(|d| d.key == "daily.total").unwrap().value, "∞");
        assert_eq!(unlimited_daily.quota_unlimited, Some(false));
        body["weekly"]["total"] = Value::Null;
        assert_eq!(summarize(&body, 0).unwrap().quota_unlimited, Some(true));
        body["daily"]["remaining"] = serde_json::json!({"bad":true});
        assert!(summarize(&body, 0).is_err());
        assert!(!matches(&serde_json::json!({"remaining":100})));
        for base in [
            "https://example.com",
            "https://example.com/v1",
            "https://example.com/v1/",
        ] {
            assert_eq!(
                usage_url(base).unwrap().as_str(),
                "https://example.com/v1/usage"
            );
        }
    }
}
