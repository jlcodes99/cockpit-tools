//! Explicit, bounded, token-free egress checks. Never retry without the selected proxy.
use serde::Serialize;
use std::{
    collections::HashMap,
    net::IpAddr,
    sync::{LazyLock, Mutex},
    time::{Duration, Instant},
};
use tokio::sync::{watch, Semaphore};

static PROBES: Semaphore = Semaphore::const_new(4);
static LATENCY_PROBES: Semaphore = Semaphore::const_new(3);

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LatencyResult {
    pub latency_ms: u64,
    pub checked_at: i64,
}

/// Explicit latency checks never use account tokens or modify active tunnels.
pub async fn latency_resource(input: String, test_url: String) -> Result<LatencyResult, String> {
    let test_url = super::codex_proxy_engine::validate_delay_url(&test_url)?;
    let _permit = LATENCY_PROBES
        .try_acquire()
        .map_err(|_| "PROXY_PROBE_BUSY")?;
    tokio::time::timeout(Duration::from_secs(10), async {
        let tunnel = crate::modules::codex_proxy_engine::start_latency(&input).await?;
        let result = tunnel.controller().measure_delay(&test_url).await;
        if result.is_err() {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        let result = result.map_err(|error| tunnel.failure(error));
        // Cancellation drops this isolated tunnel; it never touches a bound runtime.
        let _ = tunnel.stop().await;
        result.map(|latency_ms| LatencyResult {
            latency_ms,
            checked_at: chrono::Utc::now().timestamp_millis(),
        })
    })
    .await
    .map_err(|_| "PROXY_PROBE_TIMEOUT")?
}
fn request_error(error: &reqwest::Error) -> String {
    if error.is_timeout() {
        return if error.is_connect() {
            "PROXY_CONNECT_TIMEOUT"
        } else {
            "PROXY_PROBE_TIMEOUT"
        }
        .into();
    }
    let mut source: Option<&(dyn std::error::Error + 'static)> = Some(error);
    for _ in 0..8 {
        let Some(current) = source else { break };
        let text = current.to_string();
        if let Some(code) = super::codex_proxy_engine_errors::code(
            super::codex_proxy_engine_errors::classify(&text.as_bytes()[..text.len().min(4096)]),
        ) {
            return code.into();
        }
        source = current.source();
    }
    "PROXY_PROBE_FAILED".into()
}
enum ProbeEntry {
    Pending {
        account_id: String,
        created_at: Instant,
    },
    Active {
        account_id: String,
        tx: watch::Sender<bool>,
    },
}
static CANCEL: LazyLock<Mutex<HashMap<String, ProbeEntry>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProxyProbeResult {
    pub ip: String,
    pub latency_ms: u64,
    pub checked_at: i64,
    pub protocol: String,
}

struct ProbeCancel {
    request_id: String,
    tx: watch::Sender<bool>,
    rx: watch::Receiver<bool>,
}

impl ProbeCancel {
    fn register(account_id: &str, request_id: &str) -> Result<Self, String> {
        let (tx, rx) = watch::channel(false);
        let mut map = CANCEL.lock().map_err(|_| "PROXY_PROBE_FAILED")?;
        map.retain(|_, entry| !matches!(entry, ProbeEntry::Pending { created_at, .. } if created_at.elapsed() >= Duration::from_secs(30)));
        if let Some(previous) = map.remove(request_id) {
            match previous {
                ProbeEntry::Pending {
                    account_id: pending,
                    ..
                } if pending == account_id => return Err("PROXY_PROBE_CANCELLED".into()),
                ProbeEntry::Active { tx: previous, .. } => {
                    let _ = previous.send(true);
                }
                _ => {}
            }
        }
        map.insert(
            request_id.to_string(),
            ProbeEntry::Active {
                account_id: account_id.to_string(),
                tx: tx.clone(),
            },
        );
        Ok(Self {
            request_id: request_id.to_string(),
            tx,
            rx,
        })
    }
}

impl Drop for ProbeCancel {
    fn drop(&mut self) {
        if let Ok(mut map) = CANCEL.lock() {
            if map.get(&self.request_id).is_some_and(
                |entry| matches!(entry, ProbeEntry::Active { tx, .. } if tx.same_channel(&self.tx)),
            ) {
                map.remove(&self.request_id);
            }
        }
    }
}

/// Closing the dialog or pressing cancel must stop an in-flight check.
pub fn cancel(account_id: &str, request_id: &str) -> Result<(), String> {
    validate_request_id(request_id)?;
    let mut map = CANCEL.lock().map_err(|_| "PROXY_PROBE_FAILED")?;
    map.retain(|_, entry| !matches!(entry, ProbeEntry::Pending { created_at, .. } if created_at.elapsed() >= Duration::from_secs(30)));
    match map.get(request_id) {
        Some(ProbeEntry::Active {
            account_id: active,
            tx,
        }) if active == account_id => {
            let _ = tx.send(true);
        }
        Some(_) => return Err("PROXY_PROBE_FAILED".into()),
        None if map.len() < 128 => {
            map.insert(
                request_id.to_string(),
                ProbeEntry::Pending {
                    account_id: account_id.to_string(),
                    created_at: Instant::now(),
                },
            );
        }
        None => return Err("PROXY_PROBE_BUSY".into()),
    }
    Ok(())
}

fn validate_request_id(request_id: &str) -> Result<(), String> {
    if request_id.len() != 36
        || !request_id
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() || byte == b'-')
    {
        return Err("PROXY_PROBE_FAILED".into());
    }
    Ok(())
}

async fn wait_cancel(mut rx: watch::Receiver<bool>) {
    loop {
        if *rx.borrow() {
            return;
        }
        if rx.changed().await.is_err() {
            return;
        }
    }
}

pub async fn probe(
    account_id: String,
    request_id: String,
    input: Option<String>,
) -> Result<ProxyProbeResult, String> {
    validate_request_id(&request_id)?;
    let _permit = PROBES.try_acquire().map_err(|_| "PROXY_PROBE_BUSY")?;
    let cancel = ProbeCancel::register(&account_id, &request_id)?;
    tokio::select! {
        _ = wait_cancel(cancel.rx.clone()) => Err("PROXY_PROBE_CANCELLED".into()),
        result = tokio::time::timeout(Duration::from_secs(25), probe_inner(account_id, input)) =>
            result.map_err(|_| "PROXY_PROBE_TIMEOUT".to_string())?,
    }
}

async fn probe_inner(
    account_id: String,
    input: Option<String>,
) -> Result<ProxyProbeResult, String> {
    let account = crate::modules::codex_proxy_runtime::load(&account_id).await?;
    if !crate::modules::codex_account_proxy::eligible(&account) {
        return Err("PROXY_ACCOUNT_UNSUPPORTED".into());
    }
    // 探测必须使用"实际生效"的出口：统一代理开启时账号自身的绑定已经不生效。
    let input = match input {
        Some(input) => input,
        None => crate::modules::codex_account_proxy::configured_url(&account)?
            .map(|value| value.into_owned())
            .ok_or("PROXY_INVALID_URL")?,
    };
    probe_input(input).await
}

/// Resource checks use the same bounded, token-free request and global concurrency limit.
pub async fn probe_resource(input: String) -> Result<ProxyProbeResult, String> {
    let _permit = PROBES.try_acquire().map_err(|_| "PROXY_PROBE_BUSY")?;
    tokio::time::timeout(Duration::from_secs(25), probe_input(input))
        .await
        .map_err(|_| "PROXY_PROBE_TIMEOUT")?
}

async fn probe_input(input: String) -> Result<ProxyProbeResult, String> {
    let mut protocol = url::Url::parse(input.trim())
        .map_err(|_| "PROXY_INVALID_URL")?
        .scheme()
        .to_uppercase();
    if protocol == "COCKPIT-PROXY" {
        protocol = "RESOURCE".into();
    }
    let tunnel = if matches!(protocol.as_str(), "HTTP" | "HTTPS" | "SOCKS5" | "SOCKS5H") {
        None
    } else {
        Some(crate::modules::codex_proxy_engine::start(&input).await?)
    };
    let normalized = match tunnel.as_ref() {
        Some(tunnel) => tunnel.proxy_url().to_string(),
        None => crate::modules::codex_account_proxy::normalize_direct_proxy(&input)?,
    };
    let result = probe_proxy_url(&normalized, protocol).await;
    if result.is_err() {
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let result = result.map_err(|e| tunnel.as_ref().map_or(e.clone(), |t| t.failure(e)));
    if let Some(tunnel) = tunnel {
        let _ = tunnel.stop().await;
    }
    result
}

/// Optional public-IP check through the selected proxy; it never changes a binding.
async fn probe_proxy_url(normalized: &str, protocol: String) -> Result<ProxyProbeResult, String> {
    let proxy = reqwest::Proxy::all(normalized).map_err(|_| "PROXY_INVALID_URL")?;
    let client = reqwest::Client::builder()
        .no_proxy()
        .proxy(proxy)
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(12))
        .build()
        .map_err(|_| "PROXY_PROBE_FAILED")?;
    let started = Instant::now();
    let ip = tokio::time::timeout(
        Duration::from_secs(13),
        query_ip(&client, "https://api64.ipify.org?format=json"),
    ).await.map_err(|_| "PROXY_PROBE_TIMEOUT".to_string())??;
    Ok(ProxyProbeResult {
        ip,
        latency_ms: started.elapsed().as_millis() as u64,
        checked_at: chrono::Utc::now().timestamp_millis(),
        protocol,
    })
}

async fn query_ip(client: &reqwest::Client, endpoint: &str) -> Result<String, String> {
    let mut response = client
        .get(endpoint)
        .send()
        .await
        .map_err(|e| request_error(&e))?;
    if !response.status().is_success() {
        return Err("PROXY_TARGET_FAILED".into());
    }
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|_| "PROXY_PROBE_FAILED")? {
        if body.len() + chunk.len() > 8192 {
            return Err("PROXY_PROBE_RESPONSE".into());
        }
        body.extend_from_slice(&chunk);
    }
    parse_ip(&body)
}

fn parse_ip(body: &[u8]) -> Result<String, String> {
    let value: serde_json::Value =
        serde_json::from_slice(body).map_err(|_| "PROXY_PROBE_RESPONSE")?;
    let ip = value
        .get("ip")
        .and_then(|ip| ip.as_str())
        .ok_or("PROXY_PROBE_RESPONSE")?;
    ip.parse::<IpAddr>()
        .map(|ip| ip.to_string())
        .map_err(|_| "PROXY_PROBE_RESPONSE".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    #[test]
    fn latency_result_exposes_only_native_delay_and_timestamp() {
        let value = serde_json::to_value(LatencyResult {
            latency_ms: 216,
            checked_at: 123456789,
        })
        .unwrap();
        assert_eq!(
            value,
            serde_json::json!({"latencyMs":216,"checkedAt":123456789})
        );
    }

    #[tokio::test]
    async fn invalid_latency_url_is_rejected_before_engine_start() {
        for url in [
            "ftp://probe.invalid",
            "https://secret@probe.invalid",
            "https://:secret@probe.invalid",
            "invalid",
        ] {
            let result =
                latency_resource("invalid-input-that-must-not-be-parsed".into(), url.into()).await;
            assert_eq!(result.err().as_deref(), Some("PROXY_INVALID_URL"));
        }
    }

    #[tokio::test]
    async fn latency_rejects_a_fourth_probe_before_starting_an_engine() {
        let permits = LATENCY_PROBES.acquire_many(3).await.unwrap();
        let result = latency_resource(
            "invalid-input-that-must-not-be-parsed".into(),
            "http://latency.invalid/generate_204".into(),
        )
        .await;
        assert_eq!(result.err().as_deref(), Some("PROXY_PROBE_BUSY"));
        drop(permits);
    }

    #[tokio::test]
    async fn probe_uses_selected_proxy_without_account_headers() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = vec![0; 8192];
            let size = stream.read(&mut request).await.unwrap();
            let request = String::from_utf8_lossy(&request[..size]).to_lowercase();
            assert!(request.contains("http://test.invalid/"));
            assert!(!request.contains("authorization:"));
            assert!(!request.contains("chatgpt-account-id"));
            assert!(!request.contains("cookie:"));
            let body = r#"{"ip":"1.1.1.1"}"#;
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            stream.write_all(response.as_bytes()).await.unwrap();
        });
        let client = reqwest::Client::builder()
            .no_proxy()
            .proxy(reqwest::Proxy::all(format!("http://{address}")).unwrap())
            .timeout(Duration::from_secs(2))
            .build()
            .unwrap();
        assert_eq!(
            query_ip(&client, "http://test.invalid/").await.unwrap(),
            "1.1.1.1"
        );
        server.await.unwrap();
    }

    #[tokio::test]
    async fn unavailable_proxy_does_not_fall_back_to_reachable_origin() {
        let origin = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let stopped_proxy = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let proxy_address = stopped_proxy.local_addr().unwrap();
        drop(stopped_proxy);
        let client = reqwest::Client::builder()
            .no_proxy()
            .proxy(reqwest::Proxy::all(format!("http://{proxy_address}")).unwrap())
            .timeout(Duration::from_secs(1))
            .build()
            .unwrap();
        let endpoint = format!("http://{}/", origin.local_addr().unwrap());
        assert!(query_ip(&client, &endpoint).await.is_err());
        assert!(
            tokio::time::timeout(Duration::from_millis(100), origin.accept())
                .await
                .is_err()
        );
    }

    #[test]
    fn accepts_only_ip_addresses_not_remote_markup_or_errors() {
        assert_eq!(parse_ip(br#"{"ip":"1.1.1.1"}"#).unwrap(), "1.1.1.1");
        assert_eq!(
            parse_ip(br#"{"ip":"2606:4700:4700::1111"}"#).unwrap(),
            "2606:4700:4700::1111"
        );
        for body in [
            br#"{"ip":"<script>"}"#.as_slice(),
            b"proxy password error",
            br#"{"ip":5}"#,
        ] {
            assert_eq!(parse_ip(body).unwrap_err(), "PROXY_PROBE_RESPONSE");
        }
    }

    #[tokio::test]
    async fn cancel_signal_stops_waiting_without_falling_back() {
        let (tx, rx) = watch::channel(false);
        let wait = tokio::spawn(wait_cancel(rx));
        tokio::time::sleep(Duration::from_millis(20)).await;
        assert!(!wait.is_finished());
        tx.send(true).unwrap();
        tokio::time::timeout(Duration::from_secs(1), wait)
            .await
            .unwrap()
            .unwrap();
        assert!(cancel("missing-account", "11111111-2222-3333-4444-555555555555").is_ok());
        assert_eq!(
            ProbeCancel::register("missing-account", "11111111-2222-3333-4444-555555555555")
                .err()
                .as_deref(),
            Some("PROXY_PROBE_CANCELLED")
        );
        let active =
            ProbeCancel::register("active-account", "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee")
                .unwrap();
        assert!(cancel("active-account", "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee").is_ok());
        tokio::time::timeout(Duration::from_secs(1), wait_cancel(active.rx.clone()))
            .await
            .unwrap();
    }
}
