use reqwest::header::{ACCEPT, RETRY_AFTER};
use std::fmt;
use std::time::{Duration, SystemTime};

const OFFICIAL_HOST: &str = "opencode.ai";
const OFFICIAL_USAGE_PATH: &str = "/zen/go/v1/usage";
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(20);
const MAX_ERROR_BODY_BYTES: usize = 64 * 1024;

/// Errors are deliberately data-minimal: neither the API key nor upstream response bodies are kept.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OpenCodeGoHttpError {
    InvalidBaseUrl,
    InvalidApiKey,
    ClientUnavailable,
    Timeout,
    Network,
    Authentication,
    RateLimited { retry_after: Option<Duration> },
    HttpStatus(u16),
    ResponseTooLarge,
    InvalidResponse,
}

impl fmt::Display for OpenCodeGoHttpError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let code = match self {
            Self::InvalidBaseUrl => "OPENCODE_GO_BASE_URL_INVALID",
            Self::InvalidApiKey => "OPENCODE_GO_API_KEY_INVALID",
            Self::ClientUnavailable => "OPENCODE_GO_HTTP_CLIENT_UNAVAILABLE",
            Self::Timeout => "OPENCODE_GO_USAGE_TIMEOUT",
            Self::Network => "OPENCODE_GO_USAGE_NETWORK",
            Self::Authentication => "OPENCODE_GO_USAGE_AUTHENTICATION",
            Self::RateLimited { .. } => "OPENCODE_GO_USAGE_RATE_LIMIT",
            Self::HttpStatus(_) => "OPENCODE_GO_USAGE_HTTP_ERROR",
            Self::ResponseTooLarge => "OPENCODE_GO_USAGE_RESPONSE_TOO_LARGE",
            Self::InvalidResponse => "OPENCODE_GO_USAGE_INVALID_RESPONSE",
        };
        formatter.write_str(code)
    }
}

impl std::error::Error for OpenCodeGoHttpError {}

#[derive(Clone)]
pub struct OpenCodeGoHttpClient {
    client: reqwest::Client,
    endpoint: reqwest::Url,
}

impl OpenCodeGoHttpClient {
    /// Accepts only the published OpenCode Go URL shapes and always targets the
    /// canonical HTTPS usage endpoint. This prevents credentials from being sent
    /// to caller-controlled hosts, ports, paths, query strings, or userinfo.
    pub fn new(base_url: &str) -> Result<Self, OpenCodeGoHttpError> {
        let endpoint = official_usage_url(base_url)?;
        Self::with_endpoint(endpoint, DEFAULT_TIMEOUT)
    }

    fn with_endpoint(
        endpoint: reqwest::Url,
        timeout: Duration,
    ) -> Result<Self, OpenCodeGoHttpError> {
        let client = reqwest::Client::builder()
            .timeout(timeout)
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|_| OpenCodeGoHttpError::ClientUnavailable)?;
        Ok(Self { client, endpoint })
    }

    #[cfg(test)]
    pub(crate) fn for_test(endpoint: &str, timeout: Duration) -> Result<Self, OpenCodeGoHttpError> {
        let endpoint =
            reqwest::Url::parse(endpoint).map_err(|_| OpenCodeGoHttpError::InvalidBaseUrl)?;
        if endpoint.scheme() != "http" || endpoint.host_str() != Some("127.0.0.1") {
            return Err(OpenCodeGoHttpError::InvalidBaseUrl);
        }
        Self::with_endpoint(endpoint, timeout)
    }

    pub fn endpoint(&self) -> &str {
        self.endpoint.as_str()
    }

    /// Performs one GET only. A 429 is surfaced with its Retry-After hint so the
    /// caller can schedule a refresh later; automatic retries would amplify load.
    pub async fn fetch_usage(
        &self,
        api_key: &str,
    ) -> Result<serde_json::Value, OpenCodeGoHttpError> {
        let key = validate_api_key(api_key)?;
        let response = self
            .client
            .get(self.endpoint.clone())
            .bearer_auth(key)
            .header(ACCEPT, "application/json")
            .send()
            .await
            .map_err(classify_reqwest_error)?;

        let status = response.status();
        if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
            return Err(OpenCodeGoHttpError::Authentication);
        }
        if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
            return Err(OpenCodeGoHttpError::RateLimited {
                retry_after: parse_retry_after(response.headers().get(RETRY_AFTER)),
            });
        }
        if !status.is_success() {
            return Err(OpenCodeGoHttpError::HttpStatus(status.as_u16()));
        }
        if response
            .content_length()
            .is_some_and(|length| length > MAX_ERROR_BODY_BYTES as u64)
        {
            return Err(OpenCodeGoHttpError::ResponseTooLarge);
        }
        let bytes = response.bytes().await.map_err(classify_reqwest_error)?;
        if bytes.len() > MAX_ERROR_BODY_BYTES {
            return Err(OpenCodeGoHttpError::ResponseTooLarge);
        }
        serde_json::from_slice(&bytes).map_err(|_| OpenCodeGoHttpError::InvalidResponse)
    }
}

fn official_usage_url(base_url: &str) -> Result<reqwest::Url, OpenCodeGoHttpError> {
    let url =
        reqwest::Url::parse(base_url.trim()).map_err(|_| OpenCodeGoHttpError::InvalidBaseUrl)?;
    let path = url.path().trim_end_matches('/').to_ascii_lowercase();
    if url.scheme() != "https"
        || url.host_str() != Some(OFFICIAL_HOST)
        || url.port().is_some()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || !matches!(path.as_str(), "/zen/go" | "/zen/go/v1" | "/zen/go/v1/usage")
    {
        return Err(OpenCodeGoHttpError::InvalidBaseUrl);
    }
    reqwest::Url::parse(&format!("https://{OFFICIAL_HOST}{OFFICIAL_USAGE_PATH}"))
        .map_err(|_| OpenCodeGoHttpError::InvalidBaseUrl)
}

fn validate_api_key(api_key: &str) -> Result<&str, OpenCodeGoHttpError> {
    let trimmed = api_key.trim();
    if trimmed.is_empty()
        || trimmed.len() > 4096
        || trimmed
            .chars()
            .any(|character| character.is_whitespace() || character.is_control())
    {
        return Err(OpenCodeGoHttpError::InvalidApiKey);
    }
    Ok(trimmed)
}

fn classify_reqwest_error(error: reqwest::Error) -> OpenCodeGoHttpError {
    if error.is_timeout() {
        OpenCodeGoHttpError::Timeout
    } else {
        OpenCodeGoHttpError::Network
    }
}

fn parse_retry_after(value: Option<&reqwest::header::HeaderValue>) -> Option<Duration> {
    let raw = value?.to_str().ok()?.trim();
    if let Ok(seconds) = raw.parse::<u64>() {
        return Some(Duration::from_secs(seconds));
    }
    let retry_at = chrono::DateTime::parse_from_rfc2822(raw).ok()?;
    let now = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .ok()?
        .as_secs() as i64;
    let seconds = retry_at.timestamp().saturating_sub(now);
    Some(Duration::from_secs(seconds.max(0) as u64))
}

#[cfg(test)]
mod unit_tests {
    use super::*;

    #[test]
    fn retry_after_supports_delta_seconds_and_rejects_invalid_values() {
        assert_eq!(
            parse_retry_after(Some(&reqwest::header::HeaderValue::from_static("42"))),
            Some(Duration::from_secs(42))
        );
        assert_eq!(
            parse_retry_after(Some(&reqwest::header::HeaderValue::from_static("n/a"))),
            None
        );
    }

    #[test]
    fn api_key_validation_rejects_whitespace_and_control_characters() {
        assert_eq!(
            validate_api_key("  "),
            Err(OpenCodeGoHttpError::InvalidApiKey)
        );
        assert_eq!(
            validate_api_key("fixture key"),
            Err(OpenCodeGoHttpError::InvalidApiKey)
        );
        assert_eq!(validate_api_key("fixture-key"), Ok("fixture-key"));
    }
}
