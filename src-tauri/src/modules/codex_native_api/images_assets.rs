//! Untrusted image URLs never inherit provider credentials or redirect policy.
use std::{
    future::Future,
    net::{IpAddr, Ipv4Addr, SocketAddr},
    time::Duration,
};

use axum::http::header;
use base64::{engine::general_purpose::STANDARD, Engine};
use futures_util::StreamExt;
use serde_json::Value;
use tokio::time::Instant;
use crate::modules::codex_native_api::deadline::Deadline;
use url::{Host, Url};

use super::super::{contract::ServiceError, lifecycle::stopping_error, server::AppState};
use super::{ImagePart, ImageRequest, ImageResponseFormat};

pub(super) const ASSET_OPERATION_LIMIT: Duration = Duration::from_secs(5 * 60);
const DNS_LIMIT: Duration = Duration::from_secs(10);
const CONNECT_LIMIT: Duration = Duration::from_secs(30);

#[derive(Clone, Copy)]
pub(super) struct AssetBudget {
    deadline: Instant,
    idle_timeout: Duration,
}
impl AssetBudget {
    fn input(execution: &super::super::execution::Execution) -> Self {
        Self {
            deadline: execution
                .prepare_deadline()
                .min(Instant::now() + ASSET_OPERATION_LIMIT),
            idle_timeout: execution.policy.idle_timeout,
        }
    }
    pub(super) fn output(
        total_deadline: impl Into<Deadline>,
        operation_deadline: impl Into<Deadline>,
        idle_timeout: Duration,
    ) -> Self {
        Self {
            deadline: total_deadline.into()
                .min(operation_deadline)
                .cap(Instant::now() + ASSET_OPERATION_LIMIT),
            idle_timeout,
        }
    }
    fn remaining(self) -> Result<Duration, ServiceError> {
        let remaining = self.deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            Err(Self::timeout())
        } else {
            Ok(remaining)
        }
    }
    fn timeout() -> ServiceError {
        ServiceError::new(
            504,
            "image_asset_timeout",
            "Image asset operation exceeded its time limit",
        )
    }
    async fn bounded<T>(
        self,
        phase_limit: Option<Duration>,
        future: impl Future<Output = Result<T, ServiceError>>,
    ) -> Result<T, ServiceError> {
        let deadline = phase_limit.map_or(self.deadline, |limit| {
            self.deadline.min(Instant::now() + limit)
        });
        self.remaining()?;
        tokio::time::timeout_at(deadline, future)
            .await
            .map_err(|_| Self::timeout())?
    }
}

fn invalid(message: &str) -> ServiceError {
    ServiceError::new(400, "invalid_image_url", message)
}

pub(super) fn checked_url(raw: &str) -> Result<Url, ServiceError> {
    let url = Url::parse(raw).map_err(|_| invalid("Invalid image URL"))?;
    if !matches!(url.scheme(), "http" | "https")
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
    {
        return Err(invalid(
            "Image URLs must be HTTP(S) without credentials or fragments",
        ));
    }
    match url.host() {
        Some(Host::Ipv4(ip)) if public_address(IpAddr::V4(ip)) => {}
        Some(Host::Ipv6(ip)) if public_address(IpAddr::V6(ip)) => {}
        Some(Host::Domain(host))
            if !host.eq_ignore_ascii_case("localhost")
                && host.contains('.')
                && ![".localhost", ".local", ".internal", ".lan"]
                    .iter()
                    .any(|suffix| {
                        host.to_ascii_lowercase()
                            .trim_end_matches('.')
                            .ends_with(suffix)
                    }) => {}
        _ => return Err(invalid("Image URLs must use a public Internet address")),
    }
    Ok(url)
}

fn public_v4(ip: Ipv4Addr) -> bool {
    let [a, b, c, _] = ip.octets();
    !ip.is_private()
        && !ip.is_loopback()
        && !ip.is_link_local()
        && !ip.is_broadcast()
        && !ip.is_documentation()
        && !ip.is_unspecified()
        && a != 0
        && a < 224
        && !(a == 100 && (64..=127).contains(&b))
        && !(a == 198 && (b == 18 || b == 19))
        && !(a == 192 && b == 0 && c == 0)
}

fn public_address(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => public_v4(ip),
        IpAddr::V6(ip) => {
            if let Some(ip) = ip.to_ipv4_mapped() {
                return public_v4(ip);
            }
            let s = ip.segments();
            // Global unicast only. Exclude documentation, Teredo and 6to4, whose
            // embedded IPv4 destination can otherwise escape the IPv4 checks.
            s[0] & 0xe000 == 0x2000
                && s[0] != 0x2002
                && !(s[0] == 0x2001 && (s[1] <= 0x01ff || s[1] == 0x0db8))
        }
    }
}

async fn addresses(
    state: &AppState,
    url: &Url,
    budget: AssetBudget,
) -> Result<Vec<SocketAddr>, ServiceError> {
    let host = url
        .host_str()
        .ok_or_else(|| invalid("Image URL has no host"))?;
    let port = url
        .port_or_known_default()
        .ok_or_else(|| invalid("Invalid image URL port"))?;
    match url.host() {
        Some(Host::Ipv4(ip)) => return Ok(vec![SocketAddr::new(IpAddr::V4(ip), port)]),
        Some(Host::Ipv6(ip)) => return Ok(vec![SocketAddr::new(IpAddr::V6(ip), port)]),
        _ => {}
    }
    let mut stop = state.stop.clone();
    let result = tokio::select! {
        _ = stop.wait() => return Err(stopping_error()),
        result = budget.bounded(Some(DNS_LIMIT), async { tokio::net::lookup_host((host, port)).await.map_err(|_| invalid("Image host could not be resolved")) }) => result?,
    };
    let addresses: Vec<_> = result.collect();
    if addresses.is_empty()
        || addresses
            .iter()
            .any(|address| !public_address(address.ip()))
    {
        return Err(invalid(
            "Image URLs must resolve only to public Internet addresses",
        ));
    }
    Ok(addresses)
}

async fn download(
    state: &AppState,
    raw: &str,
    limit: usize,
    budget: AssetBudget,
) -> Result<(Vec<u8>, String), ServiceError> {
    let mut url = checked_url(raw)?;
    for redirects in 0..=5 {
        let resolved = addresses(state, &url, budget).await?;
        let remaining = budget.remaining()?;
        let client = reqwest::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .resolve_to_addrs(url.host_str().unwrap(), &resolved)
            .connect_timeout(remaining.min(CONNECT_LIMIT))
            .timeout(remaining)
            .build()
            .map_err(|_| invalid("Unable to prepare image download"))?;
        let mut stop = state.stop.clone();
        let response = tokio::select! {
            _ = stop.wait() => return Err(stopping_error()),
            result = budget.bounded(None, async { client.get(url.clone()).header(header::ACCEPT, "image/png,image/jpeg,image/webp,image/gif").send().await.map_err(|error| if error.is_timeout() { AssetBudget::timeout() } else { invalid("Image download failed") }) }) => result?,
        };
        if response.status().is_redirection() {
            if redirects == 5 {
                return Err(invalid("Too many image redirects"));
            }
            let location = response
                .headers()
                .get(header::LOCATION)
                .and_then(|v| v.to_str().ok())
                .ok_or_else(|| invalid("Image redirect has no valid destination"))?;
            url = checked_url(
                url.join(location)
                    .map_err(|_| invalid("Invalid image redirect"))?
                    .as_str(),
            )?;
            continue;
        }
        if !response.status().is_success() {
            return Err(invalid("Image server rejected the download"));
        }
        if response
            .content_length()
            .is_some_and(|size| size > limit as u64)
        {
            return Err(invalid(
                "Downloaded image exceeds the configured size limit",
            ));
        }
        let declared = response
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.split(';').next())
            .map(str::trim)
            .unwrap_or("")
            .to_owned();
        if !matches!(
            declared.as_str(),
            "image/png" | "image/jpeg" | "image/webp" | "image/gif" | "application/octet-stream"
        ) {
            return Err(invalid("Image server returned an unsupported content type"));
        }
        let mut stream = response.bytes_stream();
        let mut bytes = Vec::new();
        loop {
            let mut stop = state.stop.clone();
            let next = tokio::select! {
                _ = stop.wait() => return Err(stopping_error()),
                next = budget.bounded(Some(budget.idle_timeout), async { Ok(stream.next().await) }) => next?,
            };
            let Some(chunk) = next else {
                break;
            };
            let chunk = chunk.map_err(|error| {
                if error.is_timeout() {
                    AssetBudget::timeout()
                } else {
                    invalid("Image download was interrupted")
                }
            })?;
            if bytes.len().saturating_add(chunk.len()) > limit {
                return Err(invalid(
                    "Downloaded image exceeds the configured size limit",
                ));
            }
            bytes.extend_from_slice(&chunk);
        }
        let mime = sniff_mime(&bytes)
            .ok_or_else(|| invalid("Downloaded data is not a supported image"))?;
        if declared != "application/octet-stream" && declared != mime {
            return Err(invalid("Image content type does not match its bytes"));
        }
        return Ok((bytes, mime.to_owned()));
    }
    Err(invalid("Too many image redirects"))
}

fn sniff_mime(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("image/png")
    } else if bytes.starts_with(b"\xff\xd8\xff") {
        Some("image/jpeg")
    } else if bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WEBP") {
        Some("image/webp")
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        Some("image/gif")
    } else {
        None
    }
}

impl ImageRequest {
    pub(super) async fn prepare_urls(&mut self, state: &AppState) -> Result<(), ServiceError> {
        let execution = super::super::execution::current_or_new(state).await?;
        let budget = AssetBudget::input(&execution);
        let mut replacements = Vec::new();
        for reference in self.input_images.iter().chain(self.mask.iter()) {
            let Some(raw) = reference.get("image_url").and_then(Value::as_str) else {
                continue;
            };
            if raw.starts_with("data:") {
                let encoded = raw.split_once(',').map_or("", |(_, data)| data);
                let limit = state.options.max_upload_bytes.saturating_mul(4) / 3 + 4;
                if encoded.len() > limit {
                    return Err(ServiceError::new(413, "request_too_large", "Image upload exceeds its size limit"));
                }
                continue;
            }
            if replacements.iter().any(|(url, _, _)| url == raw) {
                continue;
            }
            let (bytes, mime) = download(state, raw, state.options.max_upload_bytes, budget).await?;
            replacements.push((raw.to_owned(), bytes, mime));
        }
        for (raw, bytes, mime) in replacements {
            let inline = format!("data:{mime};base64,{}", STANDARD.encode(&bytes));
            for key in ["image", "images", "mask"] {
                if let Some(value) = self.original.get_mut(key) {
                    replace_reference(value, &raw, &inline);
                }
            }
            for value in self.input_images.iter_mut().chain(self.mask.iter_mut()) {
                replace_reference(value, &raw, &inline);
            }
            if let Some(parts) = &mut self.parts {
                for part in parts.iter_mut().filter(|part| {
                    matches!(
                        part.name.as_str(),
                        "image" | "image[]" | "images" | "images[]" | "mask"
                    )
                }) {
                    if part.filename.is_none() && part.bytes == raw.as_bytes() {
                        *part = ImagePart {
                            name: part.name.clone(),
                            filename: Some("image".into()),
                            content_type: Some(mime.clone()),
                            bytes: bytes.clone(),
                        };
                    }
                }
            }
        }
        Ok(())
    }
}

fn replace_reference(value: &mut Value, raw: &str, inline: &str) {
    match value {
        Value::String(value) if value == raw => *value = inline.to_owned(),
        Value::Array(values) => values
            .iter_mut()
            .for_each(|v| replace_reference(v, raw, inline)),
        Value::Object(object) => object
            .values_mut()
            .for_each(|v| replace_reference(v, raw, inline)),
        _ => {}
    }
}

pub(super) async fn normalize_output(
    state: &AppState,
    response: &mut Value,
    format: ImageResponseFormat,
    limit: usize,
    budget: AssetBudget,
) -> Result<(), ServiceError> {
    let mime = match response.get("output_format").and_then(Value::as_str) {
        Some("jpeg" | "jpg") => "image/jpeg",
        Some("webp") => "image/webp",
        _ => "image/png",
    };
    let items = response
        .get_mut("data")
        .and_then(Value::as_array_mut)
        .ok_or_else(|| {
            ServiceError::new(
                502,
                "invalid_image_response",
                "Image response has no data array",
            )
        })?;
    if items.is_empty() {
        return Err(ServiceError::new(
            502,
            "invalid_image_response",
            "Image response has no image data",
        ));
    }
    let mut remaining = limit;
    for item in items {
        if let Some(encoded) = item.get("b64_json").and_then(Value::as_str) {
            let bytes = STANDARD.decode(encoded).map_err(|_| {
                ServiceError::new(
                    502,
                    "invalid_image_response",
                    "Image response contains invalid base64",
                )
            })?;
            if bytes.is_empty() || bytes.len() > remaining {
                return Err(ServiceError::new(
                    502,
                    "invalid_image_response",
                    "Image response exceeds its size limit",
                ));
            }
            remaining -= bytes.len();
            if format == ImageResponseFormat::Url {
                let inline = format!("data:{mime};base64,{encoded}");
                item.as_object_mut().unwrap().remove("b64_json");
                item["url"] = Value::String(inline);
            }
            continue;
        }
        let raw = item
            .get("url")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                ServiceError::new(
                    502,
                    "invalid_image_response",
                    "Image item has no URL or image bytes",
                )
            })?
            .to_owned();
        if format == ImageResponseFormat::Url && !raw.starts_with("data:") {
            checked_url(&raw).map_err(output_error)?;
            continue;
        }
        let bytes = if raw.starts_with("data:image/") {
            let (_, encoded) = raw.split_once(";base64,").ok_or_else(|| {
                ServiceError::new(502, "invalid_image_response", "Invalid image data URL")
            })?;
            STANDARD.decode(encoded).map_err(|_| {
                ServiceError::new(
                    502,
                    "invalid_image_response",
                    "Invalid image data URL bytes",
                )
            })?
        } else {
            download(state, &raw, remaining, budget)
                .await
                .map_err(output_error)?
                .0
        };
        if bytes.is_empty() || bytes.len() > remaining {
            return Err(ServiceError::new(
                502,
                "invalid_image_response",
                "Image response exceeds its size limit",
            ));
        }
        remaining -= bytes.len();
        if format == ImageResponseFormat::Base64 {
            item.as_object_mut()
                .ok_or_else(|| {
                    ServiceError::new(502, "invalid_image_response", "Invalid image result")
                })?
                .remove("url");
            item["b64_json"] = Value::String(STANDARD.encode(bytes));
        }
    }
    Ok(())
}

fn output_error(mut error: ServiceError) -> ServiceError {
    if error.status != 504 {
        error.status = 502;
    }
    error
}

pub(super) async fn normalize_responses_event(
    state: &AppState,
    event: &mut Value,
    format: ImageResponseFormat,
    budget: AssetBudget,
) -> Result<(), ServiceError> {
    if let Some(item) = event.get_mut("item") {
        normalize_result_item(state, item, format, budget).await?;
    }
    let response = if event.get("response").is_some() {
        event.get_mut("response").unwrap()
    } else {
        event
    };
    if let Some(items) = response.get_mut("output").and_then(Value::as_array_mut) {
        for item in items {
            normalize_result_item(state, item, format, budget).await?;
        }
    }
    Ok(())
}

async fn normalize_result_item(
    state: &AppState,
    item: &mut Value,
    format: ImageResponseFormat,
    budget: AssetBudget,
) -> Result<(), ServiceError> {
    if item.get("type").and_then(Value::as_str) != Some("image_generation_call") {
        return Ok(());
    }
    let Some(raw) = item
        .get("result")
        .and_then(Value::as_str)
        .filter(|value| value.starts_with("https://") || value.starts_with("http://"))
        .map(str::to_owned)
    else {
        return Ok(());
    };
    checked_url(&raw).map_err(output_error)?;
    if format == ImageResponseFormat::Base64 {
        let (bytes, _) = download(state, &raw, state.options.max_upload_bytes, budget)
            .await
            .map_err(output_error)?;
        item["result"] = Value::String(STANDARD.encode(bytes));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::super::super::{contract::ExecutionPolicy, execution::Execution};
    use super::*;

    #[tokio::test]
    async fn input_asset_preparation_uses_opening_budget_and_output_ignores_expired_opening() {
        let policy = ExecutionPolicy {
            open_timeout: Duration::from_millis(35),
            idle_timeout: Duration::from_millis(100),
            total_timeout: Duration::from_secs(1),
            ..Default::default()
        };
        let execution = Execution::from_deadline(policy, Instant::now() + Duration::from_secs(1));
        let input = AssetBudget::input(&execution);
        assert_eq!(input.deadline, execution.prepare_deadline());
        input
            .bounded(None, async {
                tokio::time::sleep(Duration::from_millis(15)).await;
                Ok(())
            })
            .await
            .unwrap();
        // Simulate inference using up its opening time before the output CDN URL
        // arrives. A download now uses the stored total deadline, not open/read.
        tokio::time::sleep(Duration::from_millis(30)).await;
        assert!(Instant::now() > execution.open_deadline);
        assert_eq!(input.remaining().unwrap_err().status, 504);
        let output = AssetBudget::output(
            execution.total_deadline,
            execution.total_deadline,
            execution.policy.idle_timeout,
        );
        output
            .bounded(Some(output.idle_timeout), async {
                tokio::time::sleep(Duration::from_millis(20)).await;
                Ok(())
            })
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn asset_download_chunks_and_redirect_hops_share_one_absolute_budget() {
        let output = AssetBudget::output(
            Instant::now() + Duration::from_millis(65),
            Instant::now() + Duration::from_secs(1),
            Duration::from_secs(1),
        );
        for _ in 0..2 {
            output
                .bounded(Some(output.idle_timeout), async {
                    tokio::time::sleep(Duration::from_millis(15)).await;
                    Ok(())
                })
                .await
                .unwrap();
        }
        let copied_for_redirect = output;
        assert_eq!(copied_for_redirect.deadline, output.deadline);
        let failure = copied_for_redirect
            .bounded(Some(copied_for_redirect.idle_timeout), async {
                tokio::time::sleep(Duration::from_millis(50)).await;
                Ok(())
            })
            .await
            .unwrap_err();
        assert_eq!(failure.code, "image_asset_timeout");
        assert_eq!(output_error(failure).status, 504);
    }

    #[tokio::test]
    async fn asset_dns_and_idle_phases_obey_their_own_limits_and_total_remaining() {
        let output = AssetBudget::output(
            Instant::now() + Duration::from_millis(150),
            Instant::now() + Duration::from_secs(1),
            Duration::from_millis(10),
        );
        let failure = output
            .bounded(Some(output.idle_timeout), async {
                tokio::time::sleep(Duration::from_millis(40)).await;
                Ok(())
            })
            .await
            .unwrap_err();
        assert_eq!(failure.status, 504);
        // Short injected DNS cap exercises the same bounded future without
        // resolving or contacting any real destination or weakening SSRF rules.
        let failure = output
            .bounded(Some(Duration::from_millis(10)), async {
                tokio::time::sleep(Duration::from_millis(40)).await;
                Ok(())
            })
            .await
            .unwrap_err();
        assert_eq!(failure.code, "image_asset_timeout");
        assert_eq!(DNS_LIMIT, Duration::from_secs(10));
        assert_eq!(CONNECT_LIMIT, Duration::from_secs(30));
    }

    #[test]
    fn assets_keep_five_minute_operation_cap_and_shorter_execution_total() {
        let now = Instant::now();
        let execution = Execution::from_deadline(
            ExecutionPolicy {
                open_timeout: Duration::from_secs(1000),
                ..Default::default()
            },
            now + Duration::from_secs(3600),
        );
        let input = AssetBudget::input(&execution);
        assert!(input.deadline <= Instant::now() + ASSET_OPERATION_LIMIT);
        assert!(input.deadline < execution.open_deadline);
        let short = now + Duration::from_millis(20);
        assert_eq!(
            AssetBudget::output(
                short,
                now + Duration::from_secs(3600),
                Duration::from_secs(1)
            )
            .deadline,
            short
        );
        let operation = now + Duration::from_millis(10);
        assert_eq!(
            AssetBudget::output(execution.total_deadline, operation, Duration::from_secs(1))
                .deadline,
            operation
        );
    }
    #[test]
    fn untrusted_image_urls_reject_private_and_ambiguous_destinations() {
        for raw in [
            "http://127.0.0.1/x",
            "http://2130706433/x",
            "http://0x7f000001/x",
            "http://10.0.0.1/x",
            "http://100.64.0.1/x",
            "http://169.254.169.254/x",
            "http://[::ffff:127.0.0.1]/x",
            "http://[2002:7f00:1::]/x",
            "http://localhost/x",
            "http://foo.local/x",
            "http://a:b@example.com/x",
            "file:///a.png",
        ] {
            assert!(checked_url(raw).is_err(), "{raw}");
        }
        assert!(checked_url("https://images.example.com/image.png?token=a").is_ok());
        assert!(checked_url("https://8.8.8.8/image.png").is_ok());
    }
    #[test]
    fn downloaded_image_mime_is_derived_from_bytes() {
        assert_eq!(sniff_mime(b"<html>"), None);
        assert_eq!(sniff_mime(b"\x89PNG\r\n\x1a\nimage"), Some("image/png"));
        assert_eq!(sniff_mime(b"GIF89aimage"), Some("image/gif"));
    }
}
