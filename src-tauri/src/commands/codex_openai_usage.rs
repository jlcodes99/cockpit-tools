//! Official organization costs and text usage, using a separate admin key.
//! No dashboard/session endpoints, inferred remaining balance or credential writes.
use super::CodexModelProviderUsageSummary;
use chrono::{DateTime, Datelike, Utc};
use reqwest::Client;
use serde_json::Value;
use std::{collections::HashSet, time::Instant};

pub(super) fn require_usage_key(key: Option<&str>) -> Result<&str, String> {
    key.map(str::trim)
        .filter(|key| !key.is_empty())
        .ok_or_else(|| "OPENAI_USAGE_NOT_CONFIGURED".into())
}

pub(super) fn is_official_base_url(base: &str) -> bool {
    reqwest::Url::parse(base.trim()).is_ok_and(|url| {
        url.scheme() == "https"
            && url.host_str() == Some("api.openai.com")
            && url.port().is_none()
            && url.username().is_empty()
            && url.password().is_none()
            && url.query().is_none()
            && url.fragment().is_none()
            && matches!(url.path(), "" | "/" | "/v1" | "/v1/")
    })
}

pub(super) async fn query(
    client: &Client,
    key: &str,
) -> Result<CodexModelProviderUsageSummary, String> {
    tokio::time::timeout(
        std::time::Duration::from_secs(20),
        query_at(client, key, "https://api.openai.com/v1", Utc::now()),
    )
    .await
    .map_err(|_| "OPENAI_USAGE_NETWORK_FAILED".to_string())?
}

fn response_error(status: u16, body: &Value) -> String {
    // Never return raw error bodies: an authentication error may echo the key.
    if status == 401 {
        return "OPENAI_USAGE_INVALID_KEY".into();
    }
    let error = body.get("error").unwrap_or(body);
    let message = error
        .as_str()
        .or_else(|| error.get("message").and_then(Value::as_str))
        .unwrap_or("");
    if status == 403
        && (message.contains("api.usage.read")
            || error.get("code").and_then(Value::as_str) == Some("insufficient_scope"))
    {
        return "OPENAI_USAGE_PERMISSION_REQUIRED".into();
    }
    format!("OPENAI_USAGE_HTTP_{status}")
}

async fn buckets(
    client: &Client,
    key: &str,
    base: &str,
    endpoint: &str,
    start: i64,
    end: i64,
) -> Result<Vec<Value>, String> {
    let mut page: Option<String> = None;
    let mut seen_pages = HashSet::new();
    let mut result = Vec::new();
    loop {
        let mut request = client
            .get(format!("{base}/organization/{endpoint}"))
            .bearer_auth(key)
            .query(&[
                ("start_time", start.to_string()),
                ("end_time", end.to_string()),
                ("bucket_width", "1d".into()),
                ("limit", "31".into()),
            ]);
        if let Some(page) = &page {
            request = request.query(&[("page", page)]);
        }
        let response = request
            .send()
            .await
            .map_err(|_| "OPENAI_USAGE_NETWORK_FAILED")?;
        let status = response.status();
        let body: Value = response.json().await.map_err(|_| {
            if status == reqwest::StatusCode::UNAUTHORIZED {
                "OPENAI_USAGE_INVALID_KEY".into()
            } else if status.is_success() {
                "OPENAI_USAGE_INVALID_RESPONSE".into()
            } else {
                format!("OPENAI_USAGE_HTTP_{}", status.as_u16())
            }
        })?;
        if !status.is_success() {
            return Err(response_error(status.as_u16(), &body));
        }
        let data = body
            .get("data")
            .and_then(Value::as_array)
            .ok_or("OPENAI_USAGE_INVALID_RESPONSE")?;
        result.extend(data.iter().cloned());
        if body.get("has_more").and_then(Value::as_bool) != Some(true) {
            break;
        }
        let next = body
            .get("next_page")
            .and_then(Value::as_str)
            .filter(|v| !v.is_empty())
            .ok_or("OPENAI_USAGE_INVALID_RESPONSE")?;
        if !seen_pages.insert(next.to_owned()) {
            return Err("OPENAI_USAGE_INVALID_RESPONSE".into());
        }
        page = Some(next.into());
    }
    Ok(result)
}

fn summarize(
    costs: &[Value],
    usage: &[Value],
    today: i64,
) -> Result<CodexModelProviderUsageSummary, String> {
    let mut month_cost = 0.0;
    let mut today_cost = 0.0;
    let mut has_month_cost = false;
    let mut has_today_cost = false;
    let mut requests = 0;
    let mut tokens = 0;
    for bucket in costs {
        let start = bucket["start_time"]
            .as_i64()
            .ok_or("OPENAI_USAGE_INVALID_RESPONSE")?;
        for row in bucket["results"]
            .as_array()
            .ok_or("OPENAI_USAGE_INVALID_RESPONSE")?
        {
            let amount = &row["amount"];
            let currency = amount["currency"]
                .as_str()
                .ok_or("OPENAI_USAGE_INVALID_RESPONSE")?;
            if !currency.eq_ignore_ascii_case("usd") {
                return Err("OPENAI_USAGE_INVALID_RESPONSE".into());
            }
            let value = amount["value"]
                .as_f64()
                .ok_or("OPENAI_USAGE_INVALID_RESPONSE")?;
            month_cost += value;
            has_month_cost = true;
            if start >= today {
                today_cost += value;
                has_today_cost = true;
            }
        }
    }
    for bucket in usage {
        for row in bucket["results"]
            .as_array()
            .ok_or("OPENAI_USAGE_INVALID_RESPONSE")?
        {
            requests += row["num_model_requests"]
                .as_i64()
                .ok_or("OPENAI_USAGE_INVALID_RESPONSE")?;
            // input_tokens already includes cached tokens; never add those twice.
            tokens += row["input_tokens"]
                .as_i64()
                .ok_or("OPENAI_USAGE_INVALID_RESPONSE")?
                + row["output_tokens"]
                    .as_i64()
                    .ok_or("OPENAI_USAGE_INVALID_RESPONSE")?;
        }
    }
    Ok(CodexModelProviderUsageSummary {
        mode: Some("openai_official".into()),
        unit: Some("USD".into()),
        // Empty cost buckets are not evidence of a reported zero charge.
        today_cost: has_today_cost.then_some(today_cost),
        total_cost: has_month_cost.then_some(month_cost),
        today_requests: Some(requests),
        today_total_tokens: Some(tokens),
        ..Default::default()
    })
}

async fn query_at(
    client: &Client,
    key: &str,
    base: &str,
    now: DateTime<Utc>,
) -> Result<CodexModelProviderUsageSummary, String> {
    let started = Instant::now();
    let today = now
        .date_naive()
        .and_hms_opt(0, 0, 0)
        .unwrap()
        .and_utc()
        .timestamp();
    let month = now
        .with_day(1)
        .unwrap()
        .date_naive()
        .and_hms_opt(0, 0, 0)
        .unwrap()
        .and_utc()
        .timestamp();
    let (costs, usage) = tokio::try_join!(
        buckets(client, key, base, "costs", month, now.timestamp()),
        buckets(
            client,
            key,
            base,
            "usage/completions",
            today,
            now.timestamp()
        ),
    )?;
    let mut summary = summarize(&costs, &usage, today)?;
    summary.latency_ms = started.elapsed().as_millis() as u64;
    Ok(summary)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn usage_requires_its_own_credential() {
        for key in [None, Some(""), Some("  ")] {
            assert_eq!(require_usage_key(key).unwrap_err(), "OPENAI_USAGE_NOT_CONFIGURED");
        }
        assert_eq!(require_usage_key(Some(" admin-key ")).unwrap(), "admin-key");
    }

    #[test]
    fn official_host_is_exact_and_does_not_capture_other_providers() {
        assert!(is_official_base_url("https://api.openai.com/v1"));
        assert!(is_official_base_url("https://api.openai.com/"));
        for url in [
            "https://api.openai.com.example/v1",
            "http://api.openai.com/v1",
            "https://example.com/v1",
            "https://user@api.openai.com/v1",
            "https://api.openai.com/custom",
            "https://api.openai.com/v1?key=x",
        ] {
            assert!(!is_official_base_url(url), "{url}");
        }
    }

    #[test]
    fn monthly_costs_are_not_a_balance_and_cached_tokens_are_not_doubled() {
        let result = summarize(&[
            json!({"start_time":1,"results":[{"amount":{"currency":"usd","value":1.25}}]}),
            json!({"start_time":2,"results":[{"amount":{"currency":"usd","value":0.75}}]}),
        ], &[json!({"results":[{"num_model_requests":4,"input_tokens":100,"output_tokens":20,"input_cached_tokens":80}]})], 2).unwrap();
        assert_eq!(result.total_cost, Some(2.0));
        assert_eq!(result.today_cost, Some(0.75));
        assert_eq!(result.today_total_tokens, Some(120));
        assert_eq!(result.today_requests, Some(4));
        assert!(
            result.balance.is_none()
                && result.remaining.is_none()
                && result.quota_remaining.is_none()
        );
        assert_eq!(summarize(&[], &[], 2).unwrap().total_cost, None);
        assert!(summarize(&[json!({"start_time":1,"results":[{}]})], &[], 2).is_err());
    }

    #[test]
    fn missing_cost_records_do_not_hide_usage_or_turn_into_zero() {
        let usage = [json!({"results":[{"num_model_requests":407,"input_tokens":83000000,"output_tokens":881000}]})];
        for costs in [vec![], vec![json!({"start_time":2,"results":[]})]] {
            let result = summarize(&costs, &usage, 2).unwrap();
            assert_eq!(result.today_cost, None);
            assert_eq!(result.total_cost, None);
            assert_eq!(result.today_requests, Some(407));
            assert_eq!(result.today_total_tokens, Some(83881000));
        }
    }

    #[test]
    fn reported_zero_and_small_costs_are_preserved_and_days_stay_separate() {
        let past = json!({"start_time":1,"results":[{"amount":{"currency":"usd","value":1.25}}]});
        let missing_today = json!({"start_time":2,"results":[]});
        let result = summarize(&[past, missing_today], &[], 2).unwrap();
        assert_eq!(result.total_cost, Some(1.25));
        assert_eq!(result.today_cost, None);
        for value in [0.0, 0.000321957] {
            let result = summarize(&[json!({"start_time":2,"results":[{"amount":{"currency":"usd","value":value}}]})], &[], 2).unwrap();
            assert_eq!(result.total_cost, Some(value));
            assert_eq!(result.today_cost, Some(value));
        }
    }

    #[test]
    fn permission_errors_are_distinct_and_credentials_never_leak() {
        assert_eq!(
            response_error(403, &json!({"error":"Missing scopes: api.usage.read"})),
            "OPENAI_USAGE_PERMISSION_REQUIRED"
        );
        assert_eq!(
            response_error(
                401,
                &json!({"error":{"message":"Invalid sk-synthetic-secret"}})
            ),
            "OPENAI_USAGE_INVALID_KEY"
        );
        assert_eq!(
            response_error(403, &json!({"error":{"message":"Country not supported"}})),
            "OPENAI_USAGE_HTTP_403"
        );
    }

    #[tokio::test]
    async fn follows_cost_pagination_and_queries_organization_text_usage() {
        use axum::{extract::Query, http::HeaderMap, routing::get, Json, Router};
        use std::collections::HashMap;
        let now: DateTime<Utc> = "2026-10-29T12:00:00Z".parse().unwrap();
        let today = now
            .date_naive()
            .and_hms_opt(0, 0, 0)
            .unwrap()
            .and_utc()
            .timestamp();
        let app = Router::new()
            .route("/v1/organization/costs", get(move |headers: HeaderMap, Query(q): Query<HashMap<String,String>>| async move {
                assert_eq!(headers["authorization"], "Bearer synthetic-key");
                assert_eq!(q["start_time"], "1790812800");
                if q.contains_key("page") {
                    assert_eq!(q["page"], "next");
                    Json(json!({"data":[{"start_time":today,"results":[{"amount":{"currency":"usd","value":0.75}}]}],"has_more":false}))
                } else {
                    Json(json!({"data":[{"start_time":1790812800,"results":[{"amount":{"currency":"usd","value":1.25}}]}],"has_more":true,"next_page":"next"}))
                }
            }))
            .route("/v1/organization/usage/completions", get(move |Query(q): Query<HashMap<String,String>>| async move {
                assert_eq!(q["start_time"], today.to_string());
                Json(json!({"data":[{"results":[{"num_model_requests":2,"input_tokens":30,"output_tokens":10}]}],"has_more":false}))
            }));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}/v1", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        let client = Client::builder().no_proxy().build().unwrap();
        let result = query_at(&client, "synthetic-key", &base, now)
            .await
            .unwrap();
        server.abort();
        assert_eq!(result.total_cost, Some(2.0));
        assert_eq!(result.today_cost, Some(0.75));
        assert_eq!(result.today_requests, Some(2));
    }
}
