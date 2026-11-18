use std::{
    collections::HashMap,
    time::{Duration, Instant},
};

use reqwest::{header, redirect, Url};
use serde::Deserialize;
use serde_json::Value;

use super::{CodexModelProviderUsageSummary, CODEX_MODEL_PROVIDER_TEST_TIMEOUT_SECS};

#[derive(Debug, Deserialize)]
pub struct CustomProviderUsage {
    pub endpoint: String,
    #[serde(default)]
    pub headers: HashMap<String, String>,
    pub mappings: HashMap<String, String>,
    pub unit: Option<String>,
}

fn endpoint_url(base: &str, endpoint: &str) -> Result<Url, String> {
    let base = Url::parse(base.trim()).map_err(|_| "PROVIDER_BASE_URL_INVALID".to_owned())?;
    let endpoint = endpoint.trim();
    if endpoint.is_empty() || !matches!(base.scheme(), "http" | "https") {
        return Err("PROVIDER_CUSTOM_USAGE_ENDPOINT_INVALID".into());
    }
    let url = base
        .join(endpoint)
        .map_err(|_| "PROVIDER_CUSTOM_USAGE_ENDPOINT_INVALID".to_owned())?;
    if url.origin() != base.origin() || !url.username().is_empty() || url.password().is_some() {
        return Err("PROVIDER_CUSTOM_USAGE_ENDPOINT_MUST_BE_SAME_ORIGIN".into());
    }
    Ok(url)
}

fn value_at_path<'a>(body: &'a Value, path: &str) -> Option<&'a Value> {
    let path = path.trim();
    if path.is_empty() {
        return None;
    }
    if path.starts_with('/') {
        return body.pointer(path);
    }
    let path = path.strip_prefix('$').unwrap_or(path);
    let path = path.replace('[', ".").replace(']', "");
    path.split('.')
        .filter(|part| !part.is_empty())
        .try_fold(body, |value, part| match value {
            Value::Array(_) => value.get(part.parse::<usize>().ok()?),
            _ => value.get(part),
        })
}

fn summarize(
    body: &Value,
    query: &CustomProviderUsage,
    latency_ms: u64,
) -> Result<CodexModelProviderUsageSummary, String> {
    let mapped = |field: &str| {
        query
            .mappings
            .get(field)?
            .split('|')
            .filter_map(|path| value_at_path(body, path))
            .find(|value| !value.is_null())
    };
    let number = |field| {
        let value = mapped(field)?;
        value
            .as_f64()
            .or_else(|| value.as_str()?.trim().parse().ok())
            .filter(|number| number.is_finite())
    };
    let integer = |field| {
        let value = mapped(field)?;
        value
            .as_i64()
            .or_else(|| value.as_str()?.trim().parse().ok())
    };
    let boolean = |field| {
        let value = mapped(field)?;
        value
            .as_bool()
            .or_else(|| value.as_str()?.trim().parse().ok())
    };
    let available = number("remaining").or_else(|| number("balance"));
    let unit = mapped("unit")
        .and_then(Value::as_str)
        .or(query.unit.as_deref())
        .unwrap_or_default()
        .trim()
        .to_owned();
    let summary = CodexModelProviderUsageSummary {
        mode: Some("custom".into()),
        is_valid: boolean("isValid"),
        remaining: available,
        balance: number("balance"),
        unit: Some(unit),
        quota_unlimited: boolean("quotaUnlimited"),
        quota_limit: number("quotaLimit"),
        quota_used: number("quotaUsed"),
        quota_remaining: number("quotaRemaining").or(available),
        today_requests: integer("todayRequests"),
        today_total_tokens: integer("todayTotalTokens"),
        today_cost: number("todayCost"),
        total_requests: integer("totalRequests"),
        total_total_tokens: integer("totalTotalTokens"),
        total_cost: number("totalCost"),
        latency_ms,
        ..Default::default()
    };
    if [
        summary.remaining,
        summary.quota_limit,
        summary.quota_used,
        summary.quota_remaining,
        summary.today_cost,
        summary.total_cost,
    ]
    .iter()
    .all(Option::is_none)
        && [
            summary.today_requests,
            summary.today_total_tokens,
            summary.total_requests,
            summary.total_total_tokens,
        ]
        .iter()
        .all(Option::is_none)
        && summary.is_valid.is_none()
        && summary.quota_unlimited.is_none()
    {
        return Err("PROVIDER_CUSTOM_USAGE_FIELDS_NOT_FOUND".into());
    }
    Ok(summary)
}

pub(super) async fn query(
    base_url: &str,
    api_key: &str,
    config: &CustomProviderUsage,
) -> Result<CodexModelProviderUsageSummary, String> {
    let api_key = api_key.trim();
    if api_key.is_empty() {
        return Err("MISSING_API_KEY".into());
    }
    let url = endpoint_url(base_url, &config.endpoint)?;
    let origin = url.origin();
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(CODEX_MODEL_PROVIDER_TEST_TIMEOUT_SECS))
        .redirect(redirect::Policy::custom(move |attempt| {
            if attempt.url().origin() == origin && attempt.previous().len() < 10 {
                attempt.follow()
            } else {
                attempt.stop()
            }
        }))
        .build()
        .map_err(|_| "CREATE_HTTP_CLIENT_FAILED".to_owned())?;
    let mut headers = header::HeaderMap::new();
    headers.insert(
        header::ACCEPT,
        header::HeaderValue::from_static("application/json"),
    );
    for (name, value) in &config.headers {
        let name = header::HeaderName::from_bytes(name.trim().as_bytes())
            .map_err(|_| "PROVIDER_CUSTOM_USAGE_HEADER_INVALID".to_owned())?;
        if name == header::HOST
            || name == header::CONTENT_LENGTH
            || name == header::TRANSFER_ENCODING
        {
            return Err("PROVIDER_CUSTOM_USAGE_HEADER_INVALID".into());
        }
        let value = header::HeaderValue::from_str(&value.replace("{{apiKey}}", api_key))
            .map_err(|_| "PROVIDER_CUSTOM_USAGE_HEADER_INVALID".to_owned())?;
        headers.insert(name, value);
    }
    let has_authorization = headers.contains_key(header::AUTHORIZATION);
    let request = client.get(url).headers(headers);
    let request = if has_authorization {
        request
    } else {
        request.bearer_auth(api_key)
    };
    let started = Instant::now();
    let response = request
        .send()
        .await
        .map_err(|_| "PROVIDER_CUSTOM_USAGE_NETWORK_FAILED".to_owned())?;
    if !response.status().is_success() {
        return Err(format!(
            "PROVIDER_CUSTOM_USAGE_HTTP_{}",
            response.status().as_u16()
        ));
    }
    let body = response
        .json::<Value>()
        .await
        .map_err(|_| "PROVIDER_CUSTOM_USAGE_JSON_INVALID".to_owned())?;
    summarize(
        &body,
        config,
        started.elapsed().as_millis().min(u64::MAX as u128) as u64,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{http::HeaderMap, response::Redirect, routing::get, Json, Router};
    use serde_json::json;
    use tokio::net::TcpListener;

    #[tokio::test]
    async fn custom_usage_queries_both_reported_endpoint_shapes() {
        let app = Router::new()
            .route(
                "/v1/usage",
                get(|headers: HeaderMap| async move {
                    assert_eq!(headers["authorization"], "Bearer fixture-key");
                    Json(json!({"remaining":null,"quota":{"remaining":"5.25"},"is_active":true}))
                }),
            )
            .route(
                "/user/balance",
                get(|headers: HeaderMap| async move {
                    assert_eq!(headers["authorization"], "Bearer fixture-key");
                    assert_eq!(headers["user-agent"], "cc-switch/1.0");
                    Json(json!({"data":[{"balance":0,"active":false,"unit":"CNY","requests":"8"}]}))
                }),
            );
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}/v1", listener.local_addr().unwrap());
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let first = serde_json::from_value(json!({
            "endpoint":"/v1/usage",
            "unit":"CNY",
            "mappings":{"remaining":"remaining | quota.remaining | balance","isValid":"is_active"}
        }))
        .unwrap();
        let summary = query(&base, "fixture-key", &first).await.unwrap();
        assert_eq!(summary.mode.as_deref(), Some("custom"));
        assert_eq!(summary.remaining, Some(5.25));
        assert_eq!(summary.unit.as_deref(), Some("CNY"));
        assert_eq!(summary.is_valid, Some(true));
        assert_eq!(summary.quota_unlimited, None);

        let second = serde_json::from_value(json!({
            "endpoint":"/user/balance",
            "headers":{"User-Agent":"cc-switch/1.0","Authorization":"Bearer {{apiKey}}"},
            "mappings":{"balance":"$.data[0].balance","unit":"/data/0/unit","isValid":"data[0].active","totalRequests":"data[0].requests"}
        })).unwrap();
        let summary = query(&base, "fixture-key", &second).await.unwrap();
        assert_eq!(summary.remaining, Some(0.0));
        assert_eq!(summary.total_requests, Some(8));
        assert_eq!(summary.unit.as_deref(), Some("CNY"));
        assert_eq!(summary.is_valid, Some(false));
        assert_eq!(summary.today_requests, None);
        assert_eq!(
            summarize(&json!({"data":[]}), &second, 0).unwrap_err(),
            "PROVIDER_CUSTOM_USAGE_FIELDS_NOT_FOUND"
        );
        server.abort();
    }

    #[tokio::test]
    async fn custom_usage_never_sends_provider_credentials_to_a_different_origin() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let other = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let destination = format!("http://{}/balance", other.local_addr().unwrap());
        let redirect_target = destination.clone();
        let app = Router::new().route(
            "/redirect",
            get(move || async move { Redirect::temporary(&redirect_target) }),
        );
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let mut config = serde_json::from_value(json!({
            "endpoint":"/redirect","headers":{"X-Relay-Key":"{{apiKey}}"},"mappings":{"remaining":"balance"}
        })).unwrap();
        assert_eq!(
            query(&base, "fixture-key", &config).await.unwrap_err(),
            "PROVIDER_CUSTOM_USAGE_HTTP_307"
        );
        config.endpoint = destination;
        assert_eq!(
            query(&base, "fixture-key", &config).await.unwrap_err(),
            "PROVIDER_CUSTOM_USAGE_ENDPOINT_MUST_BE_SAME_ORIGIN"
        );
        assert!(
            tokio::time::timeout(Duration::from_millis(30), other.accept())
                .await
                .is_err()
        );
        server.abort();
    }
}
