use reqwest::header::{ACCEPT, AUTHORIZATION};
use reqwest::Url;
use serde::Deserialize;
use serde_json::Value;
use std::time::Duration;

const QUERY_PATH: &str = "/cockpit/quota";
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_RESPONSE_BYTES: usize = 256 * 1024;
const MAX_ERROR_BODY_CHARS: usize = 300;

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexRemoteQuotaPlan {
    pub plan: String,
    pub count: i32,
    #[serde(default)]
    pub balance: Option<f64>,
    #[serde(default)]
    pub weekly_remaining_percent: Option<i32>,
    #[serde(default)]
    pub five_hour_remaining_percent: Option<i32>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexRemoteQuotaSnapshot {
    #[serde(default)]
    pub scope: Option<String>,
    #[serde(default)]
    pub weekly_remaining_percent: Option<i32>,
    #[serde(default)]
    pub five_hour_remaining_percent: Option<i32>,
    #[serde(default)]
    pub api_key_balance: Option<f64>,
    #[serde(default)]
    pub account_count: Option<i32>,
    #[serde(default)]
    pub available_account_count: Option<i32>,
    #[serde(default)]
    pub abnormal_account_count: Option<i32>,
    #[serde(default)]
    pub cooldown_account_count: Option<i32>,
    #[serde(default)]
    pub plans: Vec<CodexRemoteQuotaPlan>,
    #[serde(default)]
    pub stale: bool,
}

fn build_query_url(base_url: &str) -> Result<String, String> {
    let trimmed = base_url.trim().trim_end_matches('/');
    if trimmed.is_empty() {
        return Err("PROVIDER_BASE_URL_INVALID".to_string());
    }
    let mut url = Url::parse(trimmed).map_err(|_| "PROVIDER_BASE_URL_INVALID".to_string())?;
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
        return Err("PROVIDER_BASE_URL_INVALID".to_string());
    }
    let base_path = url.path().trim_end_matches('/');
    let api_base_path = if base_path.is_empty() { "/v1" } else { base_path };
    url.set_path(&format!(
        "{}/{}",
        api_base_path,
        QUERY_PATH.trim_start_matches('/')
    ));
    url.set_query(None);
    url.set_fragment(None);
    Ok(url.to_string())
}

fn compact_error_body(body: &str) -> String {
    body.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(MAX_ERROR_BODY_CHARS)
        .collect()
}

fn parse_snapshot(body: &[u8]) -> Result<CodexRemoteQuotaSnapshot, String> {
    let root = serde_json::from_slice::<Value>(body)
        .map_err(|error| format!("解析 Cockpit Tools 配额 JSON 失败: {}", error))?;
    let data = root
        .get("data")
        .filter(|value| value.is_object())
        .cloned()
        .unwrap_or(root);
    let payload = data
        .get("quota")
        .filter(|value| value.is_object())
        .cloned()
        .unwrap_or(data);
    serde_json::from_value(payload)
        .map_err(|error| format!("Cockpit Tools 配额响应格式不受支持: {}", error))
}

pub async fn query_quota_for_provider(
    base_url: &str,
    api_key: &str,
) -> Result<CodexRemoteQuotaSnapshot, String> {
    let key = api_key.trim();
    if key.is_empty() {
        return Err("MISSING_API_KEY".to_string());
    }
    let query_url = build_query_url(base_url)?;
    let client = reqwest::Client::builder()
        .timeout(REQUEST_TIMEOUT)
        .build()
        .map_err(|error| format!("创建 Cockpit Tools 配额客户端失败: {}", error))?;
    let response = client
        .get(&query_url)
        .header(AUTHORIZATION, format!("Bearer {}", key))
        .header(ACCEPT, "application/json")
        .send()
        .await
        .map_err(|error| format!("请求 Cockpit Tools 配额失败: {}", error))?;
    let status = response.status();
    let body = response
        .bytes()
        .await
        .map_err(|error| format!("读取 Cockpit Tools 配额响应失败: {}", error))?;

    if body.len() > MAX_RESPONSE_BYTES {
        return Err(format!(
            "Cockpit Tools 配额响应过大（{} 字节，最多 {} 字节）",
            body.len(),
            MAX_RESPONSE_BYTES
        ));
    }
    if !status.is_success() {
        let detail = compact_error_body(&String::from_utf8_lossy(&body));
        return Err(if detail.is_empty() {
            format!("Cockpit Tools 配额接口返回 HTTP {}", status.as_u16())
        } else {
            format!(
                "Cockpit Tools 配额接口返回 HTTP {}: {}",
                status.as_u16(),
                detail
            )
        });
    }

    parse_snapshot(&body)
}

#[cfg(test)]
mod tests {
    use super::{build_query_url, parse_snapshot};

    #[test]
    fn joins_v1_base_url_and_quota_path() {
        assert_eq!(
            build_query_url("http://192.168.1.20:60303/v1/").expect("query URL"),
            "http://192.168.1.20:60303/v1/cockpit/quota"
        );
    }

    #[test]
    fn adds_v1_prefix_when_only_host_is_configured() {
        assert_eq!(
            build_query_url("http://192.168.1.20:60303").expect("query URL"),
            "http://192.168.1.20:60303/v1/cockpit/quota"
        );
    }

    #[test]
    fn accepts_data_quota_envelope() {
        let snapshot = parse_snapshot(
            br#"{"data":{"quota":{"scope":"pool","accountCount":3,"availableAccountCount":3,"apiKeyBalance":123.5,"stale":false}}}"#,
        )
        .expect("quota snapshot");
        assert_eq!(snapshot.scope.as_deref(), Some("pool"));
        assert_eq!(snapshot.account_count, Some(3));
        assert_eq!(snapshot.available_account_count, Some(3));
        assert_eq!(snapshot.api_key_balance, Some(123.5));
        assert!(!snapshot.stale);
    }
}
