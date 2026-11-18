use super::*;
use crate::models::InstanceLaunchMode;
use crate::modules::{codex_instance, codex_session_resume, process};
use futures_util::future::join_all;

const ACTIVITY_SCRIPT: &str = include_str!("codex_app_enhancements/session_activity.js");
const ACTIVITY_RPC_TIMEOUT: Duration = Duration::from_secs(6);
const NAVIGATION_SCRIPT: &str = include_str!("codex_app_enhancements/session_navigation.js");
const NAVIGATION_TIMEOUT: Duration = Duration::from_secs(15);

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SelectedThread {
    thread_id: uuid::Uuid,
    focused: bool,
}

pub(super) fn local_desktop_client() -> Option<Client> {
    Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .ok()
}

async fn default_app_ports() -> Option<Vec<Option<u16>>> {
    tauri::async_runtime::spawn_blocking(|| {
        if codex_instance::load_default_settings()?.launch_mode != InstanceLaunchMode::App {
            return Ok::<_, String>(Vec::new());
        }
        let pids = process::default_codex_desktop_pids()?;
        Ok(pids
            .into_iter()
            .map(remote_debugging_port_for_pid)
            .collect())
    })
    .await
    .ok()?
    .ok()
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum DesktopActivity {
    Idle,
    Busy,
    Unknown,
}

pub(crate) async fn default_desktop_activity() -> DesktopActivity {
    use DesktopActivity::{Busy, Idle, Unknown};

    let Some(ports) = default_app_ports().await else {
        return Unknown;
    };
    if ports.is_empty() {
        return Idle;
    }

    let Some(client) = local_desktop_client() else {
        return Unknown;
    };

    let observations = join_all(ports.into_iter().map(|port| {
        let client = &client;
        async move {
            match port {
                Some(port) => instance_activity(client, port).await,
                None => Unknown,
            }
        }
    }))
    .await;
    if observations.contains(&Busy) {
        Busy
    } else if observations.iter().all(|state| *state == Idle) {
        Idle
    } else {
        Unknown
    }
}

async fn instance_activity(client: &Client, port: u16) -> DesktopActivity {
    use DesktopActivity::{Busy, Idle, Unknown};

    let targets = query_targets(client, port).await;
    let observations = join_all(
        targets
            .iter()
            .filter(|target| is_codex_app_target(target))
            .filter_map(|target| target.websocket_url.as_deref())
            .map(|url| async {
                timeout(ACTIVITY_RPC_TIMEOUT, target_activity(url))
                    .await
                    .ok()
                    .flatten()
                    .unwrap_or(Unknown)
            }),
    )
    .await;
    // The targets belong to one desktop process and ask the same local app-server.
    if observations.contains(&Busy) {
        Busy
    } else if observations.contains(&Idle) {
        Idle
    } else {
        Unknown
    }
}

async fn target_activity(url: &str) -> Option<DesktopActivity> {
    match evaluate_owned_target(url, ACTIVITY_SCRIPT)
        .await?
        .as_str()?
    {
        "idle" => Some(DesktopActivity::Idle),
        "busy" => Some(DesktopActivity::Busy),
        _ => None,
    }
}

pub(super) async fn evaluate_owned_target(url: &str, expression: &str) -> Option<Value> {
    let (mut socket, _) = timeout(CDP_CONNECT_TIMEOUT, connect_async(url))
        .await
        .ok()?
        .ok()?;
    socket
        .send(Message::Text(
            json!({
                "id": 1,
                "method": "Runtime.evaluate",
                "params": {
                    "expression": expression,
                    "returnByValue": true,
                    "awaitPromise": true,
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .ok()?;
    while let Some(message) = socket.next().await {
        let Message::Text(message) = message.ok()? else {
            continue;
        };
        let response: Value = serde_json::from_str(&message).ok()?;
        if response.get("id").and_then(Value::as_i64) == Some(1) {
            return response.pointer("/result/result/value").cloned();
        }
    }
    None
}

fn navigation_expression(mode: &str, thread: Option<uuid::Uuid>) -> String {
    format!("({NAVIGATION_SCRIPT})({}, {})", json!(mode), json!(thread))
}

pub(crate) async fn capture_default_desktop_thread() -> Option<uuid::Uuid> {
    let ports = default_app_ports().await?;
    capture_desktop_thread(ports.into_iter().flatten()).await
}

/// The outer option distinguishes a missing process from a running desktop
/// whose current view could not be identified.
pub(crate) async fn capture_before_desktop_stop(instance: &str) -> Option<Option<uuid::Uuid>> {
    let instance = instance.to_owned();
    let ports = tauri::async_runtime::spawn_blocking(move || {
        if !codex_session_resume::get(&instance)
            .ok()?
            .uses_previous_desktop_session()
        {
            return None;
        }
        let pids = if instance == crate::commands::codex_instance::DEFAULT_INSTANCE_ID {
            if codex_instance::load_default_settings().ok()?.launch_mode != InstanceLaunchMode::App
            {
                return None;
            }
            process::default_codex_desktop_pids().ok()?
        } else {
            let profile = codex_instance::load_instance_store()
                .ok()?
                .instances
                .into_iter()
                .find(|profile| {
                    profile.id == instance && profile.launch_mode == InstanceLaunchMode::App
                })?;
            vec![process::resolve_codex_pid(
                profile.last_pid,
                Some(&profile.user_data_dir),
            )?]
        };
        if pids.is_empty() {
            return None;
        }
        Some(
            pids.into_iter()
                .filter_map(remote_debugging_port_for_pid)
                .collect::<Vec<_>>(),
        )
    })
    .await
    .ok()
    .flatten()?;
    Some(capture_desktop_thread(ports).await)
}

pub(crate) async fn record_after_desktop_stop(
    instance: &str,
    selection: Option<Option<uuid::Uuid>>,
) {
    let Some(selection) = selection else { return };
    let instance = instance.to_owned();
    let result = tauri::async_runtime::spawn_blocking(move || {
        codex_session_resume::record_stopped_thread(&instance, selection)
    })
    .await
    .map_err(|error| error.to_string())
    .and_then(|result| result);
    if let Err(error) = result {
        logger::log_warn(&format!(
            "[Codex Session Resume] 记录上次桌面会话失败: {error}"
        ));
    }
}

async fn capture_desktop_thread(ports: impl IntoIterator<Item = u16>) -> Option<uuid::Uuid> {
    let client = local_desktop_client()?;
    let targets = join_all(ports.into_iter().map(|port| query_targets(&client, port))).await;
    let expression = navigation_expression("capture", None);
    let candidates = join_all(
        targets
            .iter()
            .flatten()
            .filter(|target| is_codex_app_target(target))
            .filter_map(|target| target.websocket_url.as_deref())
            .map(|url| timeout(CDP_CONNECT_TIMEOUT, evaluate_owned_target(url, &expression))),
    )
    .await;
    let candidates: Vec<SelectedThread> = candidates
        .into_iter()
        .filter_map(|result| serde_json::from_value(result.ok().flatten()?).ok())
        .collect();
    let prefer_focused = candidates.iter().any(|candidate| candidate.focused);
    let mut matches = candidates
        .into_iter()
        .filter(|candidate| !prefer_focused || candidate.focused);
    let first = matches.next()?.thread_id;
    matches
        .all(|candidate| candidate.thread_id == first)
        .then_some(first)
}

pub(crate) async fn reopen_desktop_thread_after_switch(pid: u32, thread: uuid::Uuid) {
    let Some(client) = local_desktop_client() else {
        return;
    };
    let port = tauri::async_runtime::spawn_blocking(move || remote_debugging_port_for_pid(pid))
        .await
        .ok()
        .flatten();
    let Some(port) = port else { return };
    let reopened = timeout(NAVIGATION_TIMEOUT, async {
        let open = navigation_expression("open", Some(thread));
        let confirm = navigation_expression("confirm", Some(thread));
        let mut dispatched = false;
        loop {
            if !process::is_pid_running(pid) {
                return false;
            }
            for target in query_targets(&client, port)
                .await
                .iter()
                .filter(|target| is_codex_app_target(target))
            {
                let Some(url) = target.websocket_url.as_deref() else {
                    continue;
                };
                let expression = if dispatched { &confirm } else { &open };
                let result = timeout(CDP_CONNECT_TIMEOUT, evaluate_owned_target(url, expression))
                    .await
                    .ok()
                    .flatten();
                match result.as_ref().and_then(Value::as_str) {
                    Some("opened") => return true,
                    Some("dispatched") => {
                        dispatched = true;
                        break;
                    }
                    _ => {}
                }
            }
            tokio::time::sleep(Duration::from_millis(350)).await;
        }
    })
    .await
    .unwrap_or(false);
    if reopened {
        logger::log_info("[AutoSwitch][Codex] 已打开切号前的桌面会话");
    } else {
        logger::log_warn("[AutoSwitch][Codex] 未确认重新打开切号前的桌面会话");
    }
}
