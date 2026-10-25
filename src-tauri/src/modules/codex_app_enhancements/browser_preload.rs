//! Bounded, non-pausing browser CDP registration for one explicit launch.
//! A late attachment only arms future documents. The script's document guard
//! rejects an already started document; this module never reloads or navigates.

use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::time::Duration;
use tokio::net::TcpStream;
use tokio::sync::watch;
use tokio::time::{timeout, Instant};
use tokio_tungstenite::tungstenite::{protocol::WebSocketConfig, Message};
use tokio_tungstenite::{connect_async_with_config, MaybeTlsStream, WebSocketStream};

type Socket = WebSocketStream<MaybeTlsStream<TcpStream>>;
const COMMAND: Duration = Duration::from_secs(4);
const VERSION_LIMIT: usize = 64 * 1024;

fn safe_socket(value: &str, port: u16) -> bool {
    url::Url::parse(value).is_ok_and(|url| {
        url.scheme() == "ws"
            && url.host_str() == Some("127.0.0.1")
            && url.port() == Some(port)
            && url.username().is_empty()
            && url.password().is_none()
            && url.query().is_none()
            && url.fragment().is_none()
            && url
                .path()
                .strip_prefix("/devtools/browser/")
                .is_some_and(|id| !id.is_empty() && !id.contains('/'))
    })
}

fn local_target(target: &Value) -> bool {
    if !matches!(target["type"].as_str(), Some("page" | "webview")) {
        return false;
    }
    let Some(value) = target["url"].as_str() else {
        return false;
    };
    value.is_empty()
        || value == "about:blank"
        || url::Url::parse(value).is_ok_and(|url| {
            url.scheme() == "app"
                && url.host_str() == Some("-")
                && url.path() == "/index.html"
                && url.username().is_empty()
                && url.password().is_none()
                && url.port().is_none()
        })
}

// Restrict even a future navigation of an initially blank target. The trusted
// source includes bootstrap and PRELOAD_SCRIPT, whose early-document check must
// run before any application module can be transformed.
fn guarded_source(source: &str) -> String {
    format!(";(() => {{ if (location.protocol !== 'app:' || location.hostname !== '-' || location.pathname !== '/index.html') return; if (document.documentElement) {{ window.__cockpitStartupPreload = {{ok:false,reason:'document-already-started'}}; return; }}\n{source}\n}})();")
}

fn revoke_source(owner: &str) -> String {
    let owner = json!(owner);
    format!(";(() => {{ if (location.protocol !== 'app:' || location.hostname !== '-' || location.pathname !== '/index.html') return false; const config = window.__cockpitEnhancements; if (!config || config.owner !== {owner}) return false; config.nativeFast = false; config.quotaCompat = false; config.apiService = false; config.apiKeyAuthVerified = false; config.apiDictation = false; try {{ window.__cockpitNativeFastDispose?.(); }} catch {{}} delete window.__cockpitNativeFastComplete; config.nativePatchState = 'unsupported'; config.nativePatchReason = 'browser-preload-revoked'; config.nativeQuotaPatchState = 'disabled'; config.nativeQuotaPatchReason = 'browser-preload-revoked'; return true; }})();")
}

async fn discover(client: &reqwest::Client, port: u16) -> Result<String, String> {
    let mut response = client
        .get(format!("http://127.0.0.1:{port}/json/version"))
        .send()
        .await
        .map_err(|_| "browser preload endpoint unavailable")?
        .error_for_status()
        .map_err(|_| "browser preload endpoint rejected")?;
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| "browser preload response failed")?
    {
        if bytes.len() + chunk.len() > VERSION_LIMIT {
            return Err("browser preload response too large".into());
        }
        bytes.extend_from_slice(&chunk);
    }
    let value: Value =
        serde_json::from_slice(&bytes).map_err(|_| "browser preload invalid endpoint")?;
    let socket = value["webSocketDebuggerUrl"]
        .as_str()
        .filter(|value| safe_socket(value, port))
        .ok_or("browser preload unsafe endpoint")?;
    Ok(socket.to_string())
}

#[derive(Clone)]
enum Action {
    Setup,
    Revoke {
        session: String,
    },
    Add {
        session: String,
        revision: u64,
        immediate: bool,
    },
    Remove {
        session: String,
        identifier: String,
    },
}
struct Pending {
    action: Action,
    sent: Instant,
}
struct Browser {
    socket: Socket,
    id: u64,
    pending: HashMap<u64, Pending>,
    sessions: HashSet<String>,
    prepared_sessions: HashSet<String>,
    installed: HashSet<(String, String)>,
    command_timeout: Duration,
    owner: String,
}
impl Browser {
    async fn send(
        &mut self,
        method: &str,
        params: Value,
        session: Option<&str>,
        action: Option<Action>,
    ) -> Result<(), String> {
        self.id += 1;
        let mut command = json!({"id":self.id,"method":method,"params":params});
        if let Some(session) = session {
            command["sessionId"] = json!(session);
        }
        timeout(
            self.command_timeout,
            self.socket.send(Message::Text(command.to_string().into())),
        )
        .await
        .map_err(|_| "browser preload send timed out")?
        .map_err(|_| "browser preload transport failed")?;
        if let Some(action) = action {
            self.pending.insert(
                self.id,
                Pending {
                    action,
                    sent: Instant::now(),
                },
            );
        }
        Ok(())
    }
    async fn add(
        &mut self,
        session: &str,
        source: &str,
        revision: u64,
        immediate: bool,
    ) -> Result<(), String> {
        self.prepared_sessions.insert(session.to_string());
        let mut params = json!({"source":guarded_source(source)});
        if immediate {
            params["runImmediately"] = json!(true);
        }
        self.send(
            "Page.addScriptToEvaluateOnNewDocument",
            params,
            Some(session),
            Some(Action::Add {
                session: session.into(),
                revision,
                immediate,
            }),
        )
        .await
    }
    async fn remove(&mut self, session: &str, identifier: &str) -> Result<(), String> {
        self.send(
            "Page.removeScriptToEvaluateOnNewDocument",
            json!({"identifier":identifier}),
            Some(session),
            Some(Action::Remove {
                session: session.into(),
                identifier: identifier.into(),
            }),
        )
        .await
    }
    async fn revoke(&mut self, session: &str) -> Result<(), String> {
        // Teardown only: never evaluate a new bootstrap in an existing document.
        // A page worker's unique owner means it has taken over; leave it intact.
        timeout(
            Duration::from_millis(300),
            self.send(
                "Runtime.evaluate",
                json!({"expression":revoke_source(&self.owner),"returnByValue":true,"timeout":250}),
                Some(session),
                Some(Action::Revoke {
                    session: session.into(),
                }),
            ),
        )
        .await
        .map_err(|_| "browser preload revocation timed out")?
    }
    async fn update(&mut self, source: Option<&str>, revision: u64) -> Result<(), String> {
        for (session, identifier) in self.installed.clone() {
            self.remove(&session, &identifier).await?;
        }
        for session in self.prepared_sessions.clone() {
            self.revoke(&session).await?;
        }
        if let Some(source) = source {
            for session in self.sessions.clone() {
                self.add(&session, source, revision, true).await?;
            }
        }
        Ok(())
    }
    async fn event(
        &mut self,
        event: Value,
        source: Option<&str>,
        revision: u64,
    ) -> Result<(), String> {
        if let Some(id) = event["id"].as_u64() {
            let Some(pending) = self.pending.remove(&id) else {
                return Ok(());
            };
            if event.get("error").is_some() {
                if let Action::Add {
                    session,
                    revision: requested,
                    immediate: true,
                } = &pending.action
                {
                    if event.pointer("/error/code").and_then(Value::as_i64) == Some(-32602)
                        && *requested == revision
                    {
                        if let Some(source) = source {
                            return self.add(session, source, revision, false).await;
                        }
                    }
                }
                return Err("browser preload CDP command rejected".into());
            }
            match pending.action {
                Action::Setup => {}
                Action::Revoke { .. } => {}
                Action::Remove {
                    session,
                    identifier,
                } => {
                    self.installed.remove(&(session, identifier));
                }
                Action::Add {
                    session,
                    revision: requested,
                    ..
                } => {
                    let identifier = event
                        .pointer("/result/identifier")
                        .and_then(Value::as_str)
                        .filter(|id| !id.is_empty())
                        .ok_or("browser preload missing script identifier")?
                        .to_string();
                    self.installed.insert((session.clone(), identifier.clone()));
                    if requested != revision || source.is_none() {
                        self.remove(&session, &identifier).await?;
                        if source.is_none() {
                            self.revoke(&session).await?;
                        }
                    }
                }
            }
            return Ok(());
        }
        match event["method"].as_str() {
            Some("Target.attachedToTarget") => {
                let Some(session) = event.pointer("/params/sessionId").and_then(Value::as_str)
                else {
                    return Ok(());
                };
                // We never request a pause. Resume an unexpected paused target,
                // including workers or unsupported URLs, before doing anything else.
                if event
                    .pointer("/params/waitingForDebugger")
                    .and_then(Value::as_bool)
                    == Some(true)
                {
                    self.send(
                        "Runtime.runIfWaitingForDebugger",
                        json!({}),
                        Some(session),
                        None,
                    )
                    .await?;
                }
                if !local_target(&event["params"]["targetInfo"]) {
                    self.send(
                        "Target.detachFromTarget",
                        json!({"sessionId":session}),
                        None,
                        None,
                    )
                    .await?;
                } else if self.sessions.insert(session.to_string()) {
                    if let Some(source) = source {
                        self.add(session, source, revision, true).await?;
                    }
                }
            }
            Some("Target.detachedFromTarget") => {
                if let Some(session) = event.pointer("/params/sessionId").and_then(Value::as_str) {
                    self.sessions.remove(session);
                    self.prepared_sessions.remove(session);
                    self.installed.retain(|(owner, _)| owner != session);
                    self.pending.retain(|_, pending| match &pending.action {
                        Action::Add { session: owner, .. }
                        | Action::Revoke { session: owner }
                        | Action::Remove { session: owner, .. } => owner != session,
                        Action::Setup => true,
                    });
                }
            }
            _ => {}
        }
        Ok(())
    }
    async fn cleanup(&mut self) {
        // Drain outstanding registration acknowledgements so a cancellation
        // cannot strand an acknowledged future-document script on the page.
        let deadline = Instant::now() + Duration::from_millis(250);
        while self
            .pending
            .values()
            .any(|p| matches!(p.action, Action::Add { .. }))
            && Instant::now() < deadline
        {
            let Ok(Some(Ok(Message::Text(text)))) =
                tokio::time::timeout_at(deadline, self.socket.next()).await
            else {
                break;
            };
            if let Ok(event) = serde_json::from_str::<Value>(&text) {
                let _ = self.event(event, None, u64::MAX).await;
            }
        }
        for (session, identifier) in self.installed.clone() {
            let _ = self.remove(&session, &identifier).await;
        }
        for session in self.prepared_sessions.clone() {
            let _ = self.revoke(&session).await;
        }
        let _ = self
            .send(
                "Target.setAutoAttach",
                json!({"autoAttach":false,"waitForDebuggerOnStart":false,"flatten":true}),
                None,
                None,
            )
            .await;
        let _ = self.socket.close(None).await;
    }
}

pub(super) async fn run(
    port: u16,
    source: watch::Receiver<Option<String>>,
    stop: watch::Receiver<bool>,
    remaining_launch_window: Duration,
) -> Result<(), String> {
    run_bounded(port, source, stop, remaining_launch_window, COMMAND).await
}
async fn run_bounded(
    port: u16,
    mut source: watch::Receiver<Option<String>>,
    mut stop: watch::Receiver<bool>,
    window: Duration,
    command_timeout: Duration,
) -> Result<(), String> {
    if *stop.borrow() {
        return Ok(());
    }
    let deadline = Instant::now() + window;
    let client = reqwest::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(2))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|_| "browser preload client failed")?;
    let endpoint = loop {
        tokio::select! { biased;
            _ = stop.changed() => return Ok(()),
            _ = tokio::time::sleep_until(deadline) => return Err("browser preload discovery timed out".into()),
            result = discover(&client, port) => if let Ok(endpoint) = result { break endpoint; }
        }
        tokio::select! { biased;
            _ = stop.changed() => return Ok(()),
            _ = tokio::time::sleep_until(deadline) => return Err("browser preload discovery timed out".into()),
            _ = tokio::time::sleep(Duration::from_millis(50)) => {}
        }
    };
    let config = WebSocketConfig::default()
        .max_message_size(Some(1024 * 1024))
        .max_frame_size(Some(1024 * 1024));
    let connection = tokio::select! { biased;
        _ = stop.changed() => return Ok(()),
        _ = tokio::time::sleep_until(deadline) => return Err("browser preload connection timed out".into()),
        value = timeout(command_timeout, connect_async_with_config(endpoint, Some(config), false)) => value
    };
    let (socket, _) = connection
        .map_err(|_| "browser preload connection timed out")?
        .map_err(|_| "browser preload connection failed")?;
    let mut browser = Browser {
        socket,
        id: 0,
        pending: HashMap::new(),
        sessions: HashSet::new(),
        prepared_sessions: HashSet::new(),
        installed: HashSet::new(),
        command_timeout,
        owner: format!("preload-{port}"),
    };
    let mut script = source.borrow_and_update().clone();
    let mut revision = 0;
    let operation = async {
        browser
            .send(
                "Target.setAutoAttach",
                json!({"autoAttach":true,"waitForDebuggerOnStart":false,"flatten":true}),
                None,
                Some(Action::Setup),
            )
            .await?;
        let mut tick = tokio::time::interval(Duration::from_millis(50));
        loop {
            tokio::select! { biased;
                changed = source.changed() => {
                    if changed.is_err() { break; }
                    let next = source.borrow_and_update().clone();
                    if next != script { revision += 1; script = next; browser.update(script.as_deref(), revision).await?; }
                }
                _ = tick.tick() => {
                    if browser.pending.values().any(|p| p.sent.elapsed() >= command_timeout) {
                        return Err("browser preload CDP command timed out".into());
                    }
                }
                message = browser.socket.next() => {
                    let Some(message) = message else { return Err("browser preload transport closed".into()); };
                    match message.map_err(|_| "browser preload transport failed")? {
                        Message::Text(text) => {
                            let event = serde_json::from_str(&text).map_err(|_| "browser preload invalid response")?;
                            browser.event(event, script.as_deref(), revision).await?;
                        }
                        Message::Close(_) => return Err("browser preload transport closed".into()),
                        _ => {},
                    }
                }
            }
        }
        Ok(())
    };
    let result = tokio::select! { biased;
        _ = stop.changed() => Ok(()),
        _ = tokio::time::sleep_until(deadline) => Ok(()),
        result = operation => result,
    };
    let _ = timeout(Duration::from_secs(2), browser.cleanup()).await;
    result
}

#[cfg(test)]
#[path = "browser_preload_tests.rs"]
mod tests;
