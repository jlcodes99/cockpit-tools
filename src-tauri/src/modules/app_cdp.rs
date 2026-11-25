//! Loopback DevTools transport for Cockpit-launched desktop windows.

use futures_util::{SinkExt, StreamExt};
use reqwest::Client;
use serde::{de::DeserializeOwned, Deserialize};
use serde_json::{json, Value};
use std::time::Duration;
use tokio::time::timeout;
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message;

const CDP_CONNECT_TIMEOUT: Duration = Duration::from_secs(2);

#[derive(Debug, Clone, Deserialize)]
pub(super) struct CdpTarget {
    #[serde(rename = "type")]
    pub(super) target_type: Option<String>,
    pub(super) url: Option<String>,
    pub(super) title: Option<String>,
    #[serde(rename = "webSocketDebuggerUrl")]
    websocket_url: Option<String>,
}

pub(super) fn is_safe_cdp_websocket_url(raw: &str, expected_port: u16) -> bool {
    let Ok(parsed) = reqwest::Url::parse(raw) else {
        return false;
    };
    if parsed.scheme() != "ws" || parsed.port() != Some(expected_port) {
        return false;
    }
    match parsed.host() {
        Some(url::Host::Domain(host)) => host.eq_ignore_ascii_case("localhost"),
        Some(url::Host::Ipv4(ip)) => ip.is_loopback(),
        Some(url::Host::Ipv6(ip)) => ip.is_loopback(),
        None => false,
    }
}

pub(super) async fn query_targets(
    client: &Client,
    port: u16,
    matches: impl Fn(&CdpTarget) -> bool,
) -> Option<Vec<CdpTarget>> {
    let url = format!("http://127.0.0.1:{port}/json/list");
    let response = timeout(CDP_CONNECT_TIMEOUT, client.get(url).send())
        .await
        .ok()?
        .ok()?;
    let targets = timeout(CDP_CONNECT_TIMEOUT, response.json::<Vec<CdpTarget>>())
        .await
        .ok()?
        .ok()?;
    Some(
        targets
            .into_iter()
            .filter(|target| {
                matches(target)
                    && target
                        .websocket_url
                        .as_deref()
                        .is_some_and(|url| is_safe_cdp_websocket_url(url, port))
            })
            .collect(),
    )
}

pub(super) async fn evaluate_target<T: DeserializeOwned>(
    target: &CdpTarget,
    script: &str,
    await_promise: bool,
) -> Option<T> {
    let websocket_url = target.websocket_url.as_deref()?;
    let Ok(Ok((mut socket, _))) = timeout(CDP_CONNECT_TIMEOUT, connect_async(websocket_url)).await
    else {
        return None;
    };

    let evaluate = json!({
        "id": 3,
        "method": "Runtime.evaluate",
        "params": {
            "expression": script,
            "returnByValue": true,
            "awaitPromise": await_promise
        }
    });
    if !matches!(
        timeout(
            CDP_CONNECT_TIMEOUT,
            socket.send(Message::Text(evaluate.to_string().into())),
        )
        .await,
        Ok(Ok(()))
    ) {
        return None;
    }

    let wait = if await_promise {
        Duration::from_secs(8)
    } else {
        CDP_CONNECT_TIMEOUT
    };
    timeout(wait, async {
        while let Some(Ok(message)) = socket.next().await {
            if let Message::Text(text) = message {
                if let Ok(value) = serde_json::from_str::<Value>(&text) {
                    if value.get("id").and_then(Value::as_i64) == Some(3) {
                        return serde_json::from_value(
                            value.pointer("/result/result/value")?.clone(),
                        )
                        .ok();
                    }
                }
            }
        }
        None
    })
    .await
    .ok()
    .flatten()
}
