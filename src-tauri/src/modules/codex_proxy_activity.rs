//! Opt-in, in-memory activity from account-owned Mihomo controllers.
//! Never expose controller credentials, raw engine logs, source addresses, or URLs.
use super::codex_proxy_engine::EngineController;
use crate::models::codex::CodexAccount;
use crate::modules::{codex_account_proxy, codex_proxy_runtime};
use futures::{future::join_all, StreamExt};
use serde::Serialize;
use serde_json::Value;
use std::{
    collections::{HashMap, VecDeque},
    sync::{LazyLock, Mutex},
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio::task::JoinHandle;
use uuid::Uuid;

const MAX_LOGS: usize = 300;
const MAX_CONNECTIONS: usize = 100;
const MAX_RESPONSE_BYTES: usize = 512 * 1024;
const MAX_LOG_LINE: usize = 8192;
const SUMMARY_ACCOUNT_TIMEOUT: Duration = Duration::from_secs(8);
/// 汇总每 3 秒轮询一次，读连接数时限制并发，避免同时压住多台内核。
static SUMMARY_READS: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(4);

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProxyLogEntry {
    pub id: u64,
    pub timestamp: u64,
    pub channel: &'static str,
    pub level: &'static str,
    pub network: Option<&'static str>,
    pub target: Option<String>,
    pub rule: Option<String>,
    pub outbound: Option<String>,
    pub error_code: Option<&'static str>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProxyConnection {
    pub id: String,
    pub channel: &'static str,
    pub started_at: String,
    pub network: Option<String>,
    pub target: String,
    pub chains: Vec<String>,
    pub rule: Option<String>,
    pub upload: u64,
    pub download: u64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProxyActivitySnapshot {
    pub supported: bool,
    pub enabled: bool,
    pub capture_error: bool,
    pub connections_error: bool,
    pub logs: Vec<ProxyLogEntry>,
    pub connections: Vec<ProxyConnection>,
}

/// 只含计数与字节数的汇总；不携带 target、chains、rule、日志或任何凭据。
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProxyActivitySummaryEntry {
    pub account_id: String,
    pub supported: bool,
    pub enabled: bool,
    pub connection_count: u64,
    pub upload: u64,
    pub download: u64,
}

struct CaptureTask {
    controller: EngineController,
    handle: JoinHandle<()>,
}

#[derive(Default)]
struct Capture {
    enabled: bool,
    next_id: u64,
    logs: VecDeque<ProxyLogEntry>,
    tasks: HashMap<Uuid, CaptureTask>,
    error: bool,
}

static CAPTURES: LazyLock<Mutex<HashMap<String, Capture>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

fn now_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

fn safe_field(input: &str, limit: usize) -> Option<String> {
    let value = input.trim();
    if value.is_empty()
        || value.len() > limit
        || value.chars().any(char::is_control)
        || value.contains("://")
        || value.contains(['@', '?', '#', '\\'])
        || ["token=", "password=", "apikey=", "api_key=", "secret="]
            .iter()
            .any(|key| value.to_ascii_lowercase().contains(key))
    {
        return None;
    }
    Some(value.to_owned())
}

fn safe_target(input: &str) -> Option<String> {
    let value = safe_field(input, 255)?;
    if value.contains('/') { None } else { Some(value) }
}

fn parse_log(value: &Value, channel: &'static str) -> Option<ProxyLogEntry> {
    let level = value["level"]
        .as_str()
        .or_else(|| value["type"].as_str())?;
    let level = match level {
        "info" => "info",
        "warning" | "warn" => "warning",
        "error" => "error",
        _ => return None,
    };
    let message = value["message"]
        .as_str()
        .or_else(|| value["payload"].as_str())?;
    let parsed = [("[TCP]", "TCP"), ("[UDP]", "UDP")]
        .into_iter()
        .find_map(|(prefix, network)| {
            let start = message.find(prefix)?;
            let rest = &message[start + prefix.len()..];
            let (_, route) = rest.split_once(" --> ")?;
            let (target, route) = route.split_once(" match ")?;
            let (rule, outbound) = route.split_once(" using ")?;
            Some((network, safe_target(target)?, safe_field(rule, 160)?, safe_field(outbound, 160)?))
        });
    if let Some((network, target, rule, outbound)) = parsed {
        return Some(ProxyLogEntry {
            id: 0,
            timestamp: now_millis(),
            channel,
            level,
            network: Some(network),
            target: Some(target),
            rule: Some(rule),
            outbound: Some(outbound),
            error_code: None,
        });
    }
    // Engine errors may contain URLs, credentials, or private hosts. Keep only a fixed code.
    let code = super::codex_proxy_engine_errors::classify(message.as_bytes());
    Some(ProxyLogEntry {
        id: 0,
        timestamp: now_millis(),
        channel,
        level,
        network: None,
        target: None,
        rule: None,
        outbound: None,
        error_code: Some(super::codex_proxy_engine_errors::code(code)?),
    })
}

fn append(account_id: &str, mut entry: ProxyLogEntry) {
    if let Ok(mut captures) = CAPTURES.lock() {
        if let Some(capture) = captures.get_mut(account_id).filter(|capture| capture.enabled) {
            capture.next_id = capture.next_id.wrapping_add(1);
            entry.id = capture.next_id;
            if capture.logs.len() == MAX_LOGS {
                capture.logs.pop_front();
            }
            capture.logs.push_back(entry);
            capture.error = false;
        }
    }
}

fn mark_error(account_id: &str) {
    if let Ok(mut captures) = CAPTURES.lock() {
        if let Some(capture) = captures.get_mut(account_id).filter(|capture| capture.enabled) {
            capture.error = true;
        }
    }
}

fn mark_connected(account_id: &str) {
    if let Ok(mut captures) = CAPTURES.lock() {
        if let Some(capture) = captures.get_mut(account_id).filter(|capture| capture.enabled) {
            capture.error = false;
        }
    }
}

fn client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(2))
        .build()
        .map_err(|_| "PROXY_LOG_FAILED".into())
}

async fn set_log_level(controller: &EngineController, level: &'static str) -> Result<(), String> {
    let client = client()?;
    let response = tokio::time::timeout(
        Duration::from_secs(3),
        client
            .patch(format!("{}/configs", controller.endpoint))
            .bearer_auth(&controller.secret)
            .json(&serde_json::json!({ "log-level": level }))
            .send(),
    )
    .await
    .map_err(|_| "PROXY_LOG_FAILED")?
    .map_err(|_| "PROXY_LOG_FAILED")?;
    if response.status().is_success() {
        Ok(())
    } else {
        Err("PROXY_LOG_FAILED".into())
    }
}

async fn stream_logs(account_id: String, channel: &'static str, controller: EngineController) {
    if set_log_level(&controller, "info").await.is_err() {
        mark_error(&account_id);
        return;
    }
    let Ok(client) = client() else {
        mark_error(&account_id);
        let _ = set_log_level(&controller, "warning").await;
        return;
    };
    loop {
        let response = tokio::time::timeout(
            Duration::from_secs(3),
            client
                .get(format!("{}/logs?level=info&format=structured", controller.endpoint))
                .bearer_auth(&controller.secret)
                .send(),
        )
        .await;
        if let Ok(Ok(response)) = response {
            if response.status().is_success() {
                mark_connected(&account_id);
                let mut stream = response.bytes_stream();
                let mut line = Vec::new();
                let mut overflow = false;
                while let Some(chunk) = stream.next().await {
                    let Ok(chunk) = chunk else { break };
                    for byte in chunk {
                        if byte == b'\n' {
                            if !overflow {
                                if let Ok(value) = serde_json::from_slice::<Value>(&line) {
                                    if let Some(entry) = parse_log(&value, channel) {
                                        append(&account_id, entry);
                                    }
                                }
                            }
                            line.clear();
                            overflow = false;
                        } else if line.len() < MAX_LOG_LINE {
                            line.push(byte);
                        } else {
                            overflow = true;
                        }
                    }
                }
            } else {
                mark_error(&account_id);
            }
        } else {
            mark_error(&account_id);
        }
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
}

pub fn attach_if_enabled(account_id: &str, channel: &'static str, controller: EngineController) {
    let Ok(mut captures) = CAPTURES.lock() else { return };
    let Some(capture) = captures.get_mut(account_id).filter(|capture| capture.enabled) else {
        return;
    };
    if let Some(existing) = capture.tasks.get(&controller.tunnel_id) {
        if !existing.handle.is_finished() { return; }
    }
    let handle = tokio::spawn(stream_logs(account_id.to_owned(), channel, controller.clone()));
    capture.tasks.insert(controller.tunnel_id, CaptureTask { controller, handle });
}

pub fn detach(tunnel_id: Uuid) {
    if let Ok(mut captures) = CAPTURES.lock() {
        for capture in captures.values_mut() {
            if let Some(task) = capture.tasks.remove(&tunnel_id) {
                task.handle.abort();
            }
        }
    }
}

pub fn forget(account_id: &str) {
    if let Ok(mut captures) = CAPTURES.lock() {
        if let Some(capture) = captures.remove(account_id) {
            for task in capture.tasks.into_values() {
                task.handle.abort();
            }
        }
    }
}

fn supports_activity(account: &CodexAccount) -> Result<bool, String> {
    let Some(binding) = codex_account_proxy::configured_url(account)? else {
        return Ok(false);
    };
    Ok(!url::Url::parse(binding.as_ref()).is_ok_and(|url| {
        matches!(url.scheme(), "http" | "https" | "socks5" | "socks5h")
            && url.username().is_empty()
            && url.password().is_none()
    }))
}

pub async fn set_enabled(account_id: String, enabled: bool) -> Result<(), String> {
    let account = codex_proxy_runtime::load(&account_id).await?;
    if !codex_account_proxy::eligible(&account) {
        return Err("PROXY_ACCOUNT_UNSUPPORTED".into());
    }
    if enabled && !supports_activity(&account)? {
        return Err("PROXY_LOG_UNAVAILABLE".into());
    }
    let stopped = {
        let mut captures = CAPTURES.lock().map_err(|_| "PROXY_LOG_FAILED")?;
        let capture = captures.entry(account_id.clone()).or_default();
        capture.enabled = enabled;
        capture.error = false;
        if enabled {
            Vec::new()
        } else {
            capture.tasks.drain().map(|(_, task)| task).collect::<Vec<_>>()
        }
    };
    let controllers = stopped.into_iter().map(|task| {
        task.handle.abort();
        task.controller
    }).collect::<Vec<_>>();
    // Restoring the previous level is best-effort; no core is started to do it.
    join_all(controllers.iter().map(|controller| set_log_level(controller, "warning"))).await;
    if enabled {
        let controllers = codex_proxy_runtime::active_controllers(&account_id).await;
        for (channel, controller) in controllers {
            attach_if_enabled(&account_id, channel, controller);
        }
    }
    Ok(())
}

pub fn clear(account_id: &str) {
    if let Ok(mut captures) = CAPTURES.lock() {
        if let Some(capture) = captures.get_mut(account_id) {
            capture.logs.clear();
        }
    }
}

async fn connection_payload(controller: &EngineController) -> Result<Value, String> {
    let response = tokio::time::timeout(
        Duration::from_secs(3),
        client()?
            .get(format!("{}/connections", controller.endpoint))
            .bearer_auth(&controller.secret)
            .send(),
    )
    .await
    .map_err(|_| "PROXY_LOG_FAILED")?
    .map_err(|_| "PROXY_LOG_FAILED")?;
    if !response.status().is_success() {
        return Err("PROXY_LOG_FAILED".into());
    }
    let data = tokio::time::timeout(Duration::from_secs(3), async {
        let mut data = Vec::new();
        let mut stream = response.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|_| "PROXY_LOG_FAILED")?;
            if data.len() + chunk.len() > MAX_RESPONSE_BYTES {
                return Err("PROXY_LOG_FAILED".into());
            }
            data.extend_from_slice(&chunk);
        }
        Ok::<_, String>(data)
    })
    .await
    .map_err(|_| "PROXY_LOG_FAILED")??;
    let value: Value = serde_json::from_slice(&data).map_err(|_| "PROXY_LOG_FAILED")?;
    Ok(value)
}

async fn connection_list(
    channel: &'static str,
    controller: EngineController,
) -> Result<Vec<ProxyConnection>, String> {
    parse_connections(&connection_payload(&controller).await?, channel)
}

fn parse_connections(value: &Value, channel: &'static str) -> Result<Vec<ProxyConnection>, String> {
    Ok(value["connections"]
        .as_array()
        .ok_or("PROXY_LOG_FAILED")?
        .iter()
        .take(MAX_CONNECTIONS)
        .filter_map(|entry| {
            let metadata = &entry["metadata"];
            let host = metadata["host"]
                .as_str()
                .filter(|host| !host.is_empty())
                .or_else(|| metadata["destinationIP"].as_str())?;
            let host = safe_target(host)?;
            let port = metadata["destinationPort"]
                .as_str()
                .map(str::to_owned)
                .or_else(|| metadata["destinationPort"].as_u64().map(|v| v.to_string()))?;
            let port = port.parse::<u16>().ok().filter(|port| *port > 0)?;
            Some(ProxyConnection {
                id: format!("{channel}:{}", safe_field(entry["id"].as_str()?, 80)?),
                channel,
                started_at: safe_field(entry["start"].as_str().unwrap_or(""), 80).unwrap_or_default(),
                network: safe_field(metadata["network"].as_str().unwrap_or(""), 16),
                target: format!("{host}:{port}"),
                chains: entry["chains"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|part| safe_field(part.as_str()?, 120))
                    .take(16)
                    .collect(),
                rule: safe_field(entry["rule"].as_str().unwrap_or(""), 160),
                upload: entry["upload"].as_u64().unwrap_or(0),
                download: entry["download"].as_u64().unwrap_or(0),
            })
        })
        .collect())
}

pub async fn snapshot(account_id: String) -> Result<ProxyActivitySnapshot, String> {
    let account = codex_proxy_runtime::load(&account_id).await?;
    if !codex_account_proxy::eligible(&account) {
        return Err("PROXY_ACCOUNT_UNSUPPORTED".into());
    }
    let supported = supports_activity(&account)?;
    let (enabled, capture_error, logs) = {
        let captures = CAPTURES.lock().map_err(|_| "PROXY_LOG_FAILED")?;
        captures.get(&account_id).map_or((false, false, Vec::new()), |capture| {
            (capture.enabled, capture.error, capture.logs.iter().rev().cloned().collect())
        })
    };
    let controllers = codex_proxy_runtime::active_controllers(&account_id).await;
    let mut connections = Vec::new();
    let mut connections_error = false;
    for result in join_all(controllers.into_iter().map(|(channel, controller)| {
        connection_list(channel, controller)
    })).await {
        match result {
            Ok(mut next) => connections.append(&mut next),
            Err(_) => connections_error = true,
        }
    }
    connections.truncate(MAX_CONNECTIONS);
    Ok(ProxyActivitySnapshot {
        supported,
        enabled,
        capture_error,
        connections_error,
        logs,
        connections,
    })
}

/// 汇总只保留计数与字节数，连接目标、链路、规则在聚合阶段就被丢弃。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct ConnectionTotals {
    count: u64,
    upload: u64,
    download: u64,
}

impl ConnectionTotals {
    fn merge(self, other: Self) -> Self {
        Self {
            count: self.count.saturating_add(other.count),
            upload: self.upload.saturating_add(other.upload),
            download: self.download.saturating_add(other.download),
        }
    }
}

fn aggregate_connections(value: &Value) -> ConnectionTotals {
    value["connections"]
        .as_array()
        .map(|entries| {
            entries
                .iter()
                .fold(ConnectionTotals::default(), |totals, entry| {
                    totals.merge(ConnectionTotals {
                        count: 1,
                        upload: entry["upload"].as_u64().unwrap_or(0),
                        download: entry["download"].as_u64().unwrap_or(0),
                    })
                })
        })
        .unwrap_or_default()
}

/// `None` 表示该内核读取失败。单个内核失败不清空其他内核的字节数，只标记为不可观测。
fn merge_connection_responses(responses: &[Option<Value>]) -> (bool, ConnectionTotals) {
    let mut readable = true;
    let mut totals = ConnectionTotals::default();
    for response in responses {
        match response {
            Some(value) => totals = totals.merge(aggregate_connections(value)),
            None => readable = false,
        }
    }
    (readable, totals)
}

fn summary_entry(
    account_id: &str,
    supported: bool,
    enabled: bool,
    totals: ConnectionTotals,
) -> ProxyActivitySummaryEntry {
    ProxyActivitySummaryEntry {
        account_id: account_id.to_owned(),
        supported,
        enabled,
        connection_count: totals.count,
        upload: totals.upload,
        download: totals.download,
    }
}

fn capture_enabled(account_id: &str) -> bool {
    CAPTURES
        .lock()
        .ok()
        .and_then(|captures| captures.get(account_id).map(|capture| capture.enabled))
        .unwrap_or(false)
}

/// 账号列表要读磁盘，统一放到阻塞线程池并设置超时，不占用异步运行时线程。
async fn proxy_accounts() -> Result<Vec<CodexAccount>, String> {
    let accounts = tokio::time::timeout(
        SUMMARY_ACCOUNT_TIMEOUT,
        tauri::async_runtime::spawn_blocking(crate::modules::codex_account::list_accounts),
    )
    .await
    .map_err(|_| "PROXY_ACTIVITY_SUMMARY_TIMEOUT")?
    .map_err(|_| "PROXY_ACTIVITY_SUMMARY_FAILED")?;
    Ok(accounts
        .into_iter()
        .filter(codex_account_proxy::eligible)
        .collect())
}

/// 单个账号的任何失败都退化为「0 计数 + supported=false」，绝不中断整份汇总。
async fn account_summary(account: CodexAccount) -> ProxyActivitySummaryEntry {
    let enabled = capture_enabled(&account.id);
    if codex_proxy_runtime::ensure_account_proxy_state(&account).await.is_err()
        || !supports_activity(&account).unwrap_or(false) {
        return summary_entry(&account.id, false, enabled, ConnectionTotals::default());
    }
    let Ok(_permit) = SUMMARY_READS.acquire().await else {
        return summary_entry(&account.id, false, enabled, ConnectionTotals::default());
    };
    let controllers = codex_proxy_runtime::active_controllers(&account.id).await;
    let responses = join_all(controllers.into_iter().map(|(_channel, controller)| async move {
        connection_payload(&controller).await.ok()
    }))
    .await;
    let (readable, totals) = merge_connection_responses(&responses);
    summary_entry(&account.id, readable, enabled, totals)
}

pub async fn summary() -> Result<Vec<ProxyActivitySummaryEntry>, String> {
    let accounts = proxy_accounts().await?;
    Ok(join_all(accounts.into_iter().map(account_summary)).await)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_route_fields_or_fixed_errors_escape_the_parser() {
        let route = serde_json::json!({"level":"info","message":"[TCP] 127.0.0.1:54321 --> api.example.com:443 match MATCH using US-node"});
        let event = parse_log(&route, "account").unwrap();
        assert_eq!(event.target.as_deref(), Some("api.example.com:443"));
        assert_eq!(event.outbound.as_deref(), Some("US-node"));
        assert!(parse_log(&serde_json::json!({"type":"info","payload":"token=secret"}), "account").is_none());
        let error = parse_log(&serde_json::json!({"level":"error","message":"handshake failed token=secret"}), "desktop").unwrap();
        assert_eq!(error.error_code, Some("PROXY_HANDSHAKE_FAILED"));
        assert!(!serde_json::to_string(&error).unwrap().contains("secret"));
        assert!(parse_log(&serde_json::json!({"level":"info","message":"[TCP] x --> https://token@example.com:443 match MATCH using node"}), "account").is_none());
        let warning = parse_log(&serde_json::json!({"level":"warn","message":"[UDP] mihomo --> 223.5.5.5:53 match MATCH using US / Residential"}), "sidecar").unwrap();
        assert_eq!(warning.level, "warning");
        assert_eq!(warning.outbound.as_deref(), Some("US / Residential"));
        assert!(parse_log(&serde_json::json!({"level":"info","message":"[TCP] x --> api.example.com/path match MATCH using node"}), "account").is_none());
    }

    #[test]
    fn active_connections_keep_only_bounded_route_metadata() {
        let response = serde_json::json!({"connections": [
            {"id":"id-1", "start":"2026-09-24T10:00:00Z", "metadata":{
                "host":"api.example.com", "destinationPort":"443", "network":"tcp",
                "sourceIP":"192.0.2.1", "processName":"private-process"},
             "chains":["US / Residential", "account-node"], "rule":"MATCH",
             "upload":1024, "download":2048},
            {"id":"id-2", "metadata":{"host":"https://user:secret@example.com/path", "destinationPort":"443"}, "chains":[], "rule":"MATCH"}
        ]});
        let connections = parse_connections(&response, "account").unwrap();
        assert_eq!(connections.len(), 1);
        assert_eq!(connections[0].target, "api.example.com:443");
        assert_eq!(connections[0].chains[0], "US / Residential");
        let exported = serde_json::to_string(&connections).unwrap();
        assert!(!exported.contains("private-process"));
        assert!(!exported.contains("192.0.2.1"));
        assert!(!exported.contains("secret"));
    }

    /// 未运行、不支持或读取失败的账号只返回零计数，不抛出错误、不影响其他账号。
    #[test]
    fn unsupported_or_idle_accounts_report_zero_without_errors() {
        let (readable, totals) = merge_connection_responses(&[]);
        assert!(readable);
        assert_eq!(totals, ConnectionTotals::default());
        let idle = summary_entry("idle-account", readable, false, totals);
        assert!(idle.supported);
        assert!(!idle.enabled);
        assert_eq!(idle.connection_count, 0);
        assert_eq!(idle.upload + idle.download, 0);

        let unsupported = summary_entry("api-key-account", false, false, ConnectionTotals::default());
        assert!(!unsupported.supported);
        assert_eq!(unsupported.connection_count, 0);

        let (readable, totals) = merge_connection_responses(&[None]);
        assert!(!readable);
        let failed = summary_entry("broken-account", readable, true, totals);
        assert!(!failed.supported);
        assert_eq!(failed.connection_count, 0);
        assert_eq!(failed.upload, 0);
    }

    /// 多内核、多连接的字节数与连接数按内核分别读取后求和，并做饱和累加。
    #[test]
    fn summary_aggregates_bytes_and_connection_counts() {
        let account_engine = serde_json::json!({"connections": [
            {"id":"a","metadata":{"host":"api.example.com","destinationPort":"443"},"upload":1024,"download":2048},
            {"id":"b","metadata":{"host":"cdn.example.com","destinationPort":"443"},"upload":512,"download":256}
        ]});
        let desktop_engine = serde_json::json!({"connections": [{"id":"c","upload":1,"download":2}]});
        let (readable, totals) =
            merge_connection_responses(&[Some(account_engine), Some(desktop_engine)]);
        assert!(readable);
        assert_eq!(totals.count, 3);
        assert_eq!(totals.upload, 1537);
        assert_eq!(totals.download, 2306);
        let entry = summary_entry("account-1", readable, true, totals);
        assert_eq!(entry.connection_count, 3);
        assert_eq!(entry.upload, 1537);
        assert_eq!(entry.download, 2306);

        let maxed = serde_json::json!({"connections": [{"upload": u64::MAX, "download": u64::MAX}]});
        let totals = merge_connection_responses(&[Some(maxed.clone()), Some(maxed)]).1;
        assert_eq!(totals.upload, u64::MAX);
        assert_eq!(totals.download, u64::MAX);
    }

    /// 汇总结果只有计数字段，target、chains、rule 与来源地址都不会出现在命令返回值里。
    #[test]
    fn summary_never_exposes_connection_targets() {
        let payload = serde_json::json!({"connections": [{
            "id": "a",
            "metadata": {
                "host": "private.example.com",
                "destinationPort": "8443",
                "sourceIP": "192.0.2.9",
                "processName": "secret-app"
            },
            "chains": ["US / Residential"],
            "rule": "DOMAIN-SUFFIX",
            "upload": 10,
            "download": 20
        }]});
        let (readable, totals) = merge_connection_responses(&[Some(payload)]);
        let entry = summary_entry("account-1", readable, true, totals);
        let exported = serde_json::to_string(&entry).unwrap();
        assert_eq!(
            serde_json::to_value(&entry).unwrap(),
            serde_json::json!({
                "accountId": "account-1",
                "supported": true,
                "enabled": true,
                "connectionCount": 1,
                "upload": 10,
                "download": 20
            })
        );
        for leaked in [
            "private.example.com",
            "192.0.2.9",
            "secret-app",
            "US / Residential",
            "DOMAIN-SUFFIX",
            "target",
            "chains",
            "rule",
        ] {
            assert!(!exported.contains(leaked), "summary leaked {leaked}");
        }
    }
}
