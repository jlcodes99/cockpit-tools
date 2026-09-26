//! The engine's URLTest result is the latency, not the host's controller RTT.
use super::EngineController;
use std::time::Duration;

const DELAY_TIMEOUT: Duration = Duration::from_secs(8);
const RESPONSE_LIMIT: usize = 8192;

pub(crate) fn validate_delay_url(value: &str) -> Result<String, String> {
    if value.len() > 4096 {
        return Err("PROXY_INVALID_URL".into());
    }
    let url = url::Url::parse(value).map_err(|_| "PROXY_INVALID_URL")?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err("PROXY_INVALID_URL".into());
    }
    Ok(url.into())
}

impl EngineController {
    pub(crate) async fn measure_delay(&self, test_url: &str) -> Result<u64, String> {
        query_delay(self, test_url, DELAY_TIMEOUT).await
    }
}

async fn query_delay(
    controller: &EngineController,
    test_url: &str,
    timeout: Duration,
) -> Result<u64, String> {
    let test_url = validate_delay_url(test_url)?;
    let deadline = timeout + Duration::from_millis(500);
    // Authenticated controller traffic must never inherit a system/environment
    // proxy or follow a redirect that could disclose the random local secret.
    let client = reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(1))
        .timeout(deadline)
        .build()
        .map_err(|_| "PROXY_PROBE_FAILED")?;
    tokio::time::timeout(deadline, async {
        let mut response = client
            .get(format!(
                "{}/proxies/account-node/delay",
                controller.endpoint
            ))
            .bearer_auth(&controller.secret)
            .query(&[
                ("url", test_url),
                ("timeout", timeout.as_millis().to_string()),
            ])
            .send()
            .await
            .map_err(delay_request_error)?;
        match response.status() {
            reqwest::StatusCode::OK => {}
            reqwest::StatusCode::GATEWAY_TIMEOUT | reqwest::StatusCode::REQUEST_TIMEOUT => {
                return Err("PROXY_PROBE_TIMEOUT".into());
            }
            _ => return Err("PROXY_PROBE_FAILED".into()),
        }
        if response
            .content_length()
            .is_some_and(|size| size > RESPONSE_LIMIT as u64)
        {
            return Err("PROXY_PROBE_RESPONSE".into());
        }
        let mut body = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(delay_request_error)? {
            if body.len() + chunk.len() > RESPONSE_LIMIT {
                return Err("PROXY_PROBE_RESPONSE".into());
            }
            body.extend_from_slice(&chunk);
        }
        let value: serde_json::Value =
            serde_json::from_slice(&body).map_err(|_| "PROXY_PROBE_RESPONSE")?;
        value["delay"]
            .as_u64()
            .filter(|delay| *delay > 0 && *delay <= u16::MAX as u64)
            .ok_or_else(|| "PROXY_PROBE_RESPONSE".into())
    })
    .await
    .map_err(|_| "PROXY_PROBE_TIMEOUT".to_string())?
}

fn delay_request_error(error: reqwest::Error) -> String {
    // Controller errors can embed URLs or returned data. Only stable codes leave
    // this boundary; neither the URL nor the controller secret is exposed.
    if error.is_timeout() {
        "PROXY_PROBE_TIMEOUT"
    } else {
        "PROXY_PROBE_FAILED"
    }
    .into()
}

#[cfg(test)]
#[path = "codex_proxy_engine_delay_tests.rs"]
mod tests;
