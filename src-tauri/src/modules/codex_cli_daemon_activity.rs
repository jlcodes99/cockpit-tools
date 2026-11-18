use super::{connect_control_socket, RestartOutcome, WORKSPACE_MISMATCH};
use crate::modules::codex_account::CodexOfficialOAuthIdentity;
use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use std::{collections::HashSet, path::Path, time::Duration};
use tokio::net::UnixStream;
use tokio_tungstenite::{client_async, tungstenite::Message, WebSocketStream};

pub(crate) async fn managed_daemon_is_idle(home: &Path, expected_pid: u32) -> bool {
    tokio::time::timeout(Duration::from_secs(6), async {
        let socket = connect_owned_daemon(home, expected_pid).await?;
        read_idle_status(socket).await
    })
    .await
    .ok()
    .flatten()
        == Some(true)
}

pub(super) async fn managed_daemon_is_ready(home: &Path, expected_pid: u32) -> bool {
    tokio::time::timeout(Duration::from_secs(6), async {
        let mut socket = connect_owned_daemon(home, expected_pid).await?;
        let mut id = 0;
        initialize(&mut socket, &mut id).await
    })
    .await
    .ok()
    .flatten()
    .is_some()
}

#[cfg(target_os = "macos")]
pub(super) async fn remote_control_is_disabled(home: &Path, expected_pid: u32) -> bool {
    tokio::time::timeout(Duration::from_secs(3), async {
        let mut socket = connect_owned_daemon(home, expected_pid).await?;
        let mut id = 0;
        initialize(&mut socket, &mut id).await?;
        request(
            &mut socket,
            &mut id,
            "remoteControl/status/read",
            Value::Null,
        )
        .await
        .map(|result| result.get("status").and_then(Value::as_str) == Some("disabled"))
    })
    .await
    .ok()
    .flatten()
        == Some(true)
}

pub(super) async fn verify_workspace(
    home: &Path,
    expected_pid: u32,
    identity: &CodexOfficialOAuthIdentity,
) -> Result<RestartOutcome, String> {
    if identity.account_id.is_none() {
        return Ok(RestartOutcome::WorkspaceUnconfirmed);
    }
    let result = tokio::time::timeout(Duration::from_secs(6), async {
        let mut socket = connect_owned_daemon(home, expected_pid).await?;
        let mut id = 0;
        initialize(&mut socket, &mut id).await?;
        request(
            &mut socket,
            &mut id,
            "account/read",
            json!({ "refreshToken": false }),
        )
        .await
    })
    .await
    .ok()
    .flatten();
    match result {
        Some(result) => compare_workspace(identity, &result),
        None => Ok(RestartOutcome::WorkspaceUnconfirmed),
    }
}

fn compare_workspace(
    identity: &CodexOfficialOAuthIdentity,
    response: &Value,
) -> Result<RestartOutcome, String> {
    let observed_workspace = response
        .pointer("/workspaceRouting/chatgptAccountId")
        .and_then(Value::as_str)
        .filter(|id| !id.is_empty());
    let observed_user = response
        .get("userId")
        .and_then(Value::as_str)
        .filter(|id| !id.is_empty());
    let workspace = identity.account_id.as_deref().zip(observed_workspace);
    let user = identity.user_id.as_deref().zip(observed_user);
    let account_type = response.pointer("/account/type").and_then(Value::as_str);
    if account_type.is_some_and(|kind| kind != "chatgpt")
        || workspace.is_some_and(|(expected, got)| expected != got)
        || user.is_some_and(|(expected, got)| expected != got)
    {
        return Err(WORKSPACE_MISMATCH.into());
    }
    if account_type == Some("chatgpt")
        && workspace.is_some()
        && (identity.user_id.is_none() || user.is_some())
    {
        return Ok(RestartOutcome::WorkspaceConfirmed);
    }
    Ok(RestartOutcome::WorkspaceUnconfirmed)
}

async fn connect_owned_daemon(
    home: &Path,
    expected_pid: u32,
) -> Option<WebSocketStream<UnixStream>> {
    let stream = connect_control_socket(home).await.ok()?;
    let peer = stream.peer_cred().ok()?.pid()?;
    if u32::try_from(peer).ok()? != expected_pid {
        return None;
    }
    client_async("ws://localhost/rpc", stream)
        .await
        .ok()
        .map(|(socket, _)| socket)
}

async fn request(
    socket: &mut WebSocketStream<UnixStream>,
    id: &mut u64,
    method: &str,
    params: Value,
) -> Option<Value> {
    *id += 1;
    socket
        .send(Message::Text(
            json!({ "id": *id, "method": method, "params": params })
                .to_string()
                .into(),
        ))
        .await
        .ok()?;
    while let Some(message) = socket.next().await {
        match message.ok()? {
            Message::Text(text) => {
                let reply: Value = serde_json::from_str(&text).ok()?;
                if reply.get("id").and_then(Value::as_u64) == Some(*id) {
                    return reply.get("result").cloned();
                }
            }
            Message::Close(_) => return None,
            _ => {}
        }
    }
    None
}

async fn initialize(socket: &mut WebSocketStream<UnixStream>, id: &mut u64) -> Option<()> {
    request(
        socket,
        id,
        "initialize",
        json!({
            "clientInfo": {"name": "cockpit-idle-probe", "version": env!("CARGO_PKG_VERSION")},
            "capabilities": {"experimentalApi": true}
        }),
    )
    .await?;
    socket
        .send(Message::Text(r#"{"method":"initialized"}"#.into()))
        .await
        .ok()?;
    Some(())
}

async fn read_idle_status(mut socket: WebSocketStream<UnixStream>) -> Option<bool> {
    let mut id = 0;
    initialize(&mut socket, &mut id).await?;
    let mut cursor = None::<String>;
    let mut visited = HashSet::new();
    loop {
        let page = request(
            &mut socket,
            &mut id,
            "thread/loaded/list",
            json!({ "cursor": cursor, "limit": 100 }),
        )
        .await?;
        for entry in page.get("data")?.as_array()? {
            let thread_id = entry.as_str().filter(|id| !id.is_empty())?;
            let result = request(
                &mut socket,
                &mut id,
                "thread/read",
                json!({ "threadId": thread_id, "includeTurns": false }),
            )
            .await?;
            if result.pointer("/thread/id")?.as_str()? != thread_id {
                return None;
            }
            match result.pointer("/thread/status/type")?.as_str()? {
                "idle" => {}
                "active" => return Some(false),
                _ => return None,
            }
        }
        match page.get("nextCursor") {
            None | Some(Value::Null) => return Some(true),
            Some(Value::String(next)) if !next.is_empty() && visited.insert(next.clone()) => {
                cursor = Some(next.clone());
            }
            _ => return None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::net::UnixListener;
    use tokio_tungstenite::accept_async;

    #[test]
    fn matching_email_cannot_hide_a_different_workspace_or_user() {
        let expected = CodexOfficialOAuthIdentity {
            email: "same@example.invalid".into(),
            account_id: Some("workspace-b".into()),
            user_id: Some("user-a".into()),
            organization_id: None,
        };
        let outcome = |workspace, user| {
            compare_workspace(
                &expected,
                &json!({
                    "account": {"type":"chatgpt","email":"same@example.invalid"},
                    "workspaceRouting":{"chatgptAccountId":workspace},
                    "userId":user,
                }),
            )
        };
        assert!(matches!(
            outcome(Some("workspace-b"), Some("user-a")),
            Ok(RestartOutcome::WorkspaceConfirmed)
        ));
        assert!(matches!(
            outcome(None, Some("user-a")),
            Ok(RestartOutcome::WorkspaceUnconfirmed)
        ));
        assert!(matches!(
            outcome(Some("workspace-b"), None),
            Ok(RestartOutcome::WorkspaceUnconfirmed)
        ));
        assert_eq!(
            outcome(Some("workspace-a"), Some("user-a"))
                .err()
                .as_deref(),
            Some(WORKSPACE_MISMATCH)
        );
        assert_eq!(
            outcome(Some("workspace-b"), Some("user-b"))
                .err()
                .as_deref(),
            Some(WORKSPACE_MISMATCH)
        );
    }

    #[tokio::test]
    async fn requires_the_daemon_owner_and_checks_all_loaded_pages() {
        for (status, cursor, owner_matches, allowed) in [
            ("idle", Value::Null, true, true),
            ("active", Value::Null, true, false),
            ("notLoaded", Value::Null, true, false),
            ("idle", json!("more"), true, false),
            ("idle", Value::Null, false, false),
        ] {
            let fixture = tempfile::Builder::new()
                .prefix("cda")
                .tempdir_in("/tmp")
                .unwrap();
            let path = fixture.path().join(super::super::CONTROL_SOCKET);
            std::fs::create_dir(path.parent().unwrap()).unwrap();
            let listener = UnixListener::bind(path).unwrap();
            let rpc = tokio::spawn(async move {
                let (peer, _) = listener.accept().await.unwrap();
                let socket = accept_async(peer).await;
                if !owner_matches {
                    assert!(
                        socket.is_err(),
                        "a different PID must receive no WebSocket request"
                    );
                    return;
                }
                let mut socket = socket.unwrap();
                let mut round = 0;
                while let Some(Ok(Message::Text(message))) = socket.next().await {
                    let rpc: Value = serde_json::from_str(&message).unwrap();
                    let result = match rpc["method"].as_str().unwrap() {
                        "initialize" => json!({}),
                        "initialized" => continue,
                        "thread/loaded/list" => {
                            round += 1;
                            if round == 1 {
                                assert!(rpc["params"]["cursor"].is_null());
                                json!({"data":["first"], "nextCursor":"more"})
                            } else {
                                assert_eq!(rpc["params"]["cursor"], "more");
                                json!({"data":["second"], "nextCursor":cursor})
                            }
                        }
                        "thread/read" => {
                            assert_eq!(rpc["params"]["includeTurns"], false);
                            let thread = rpc["params"]["threadId"].as_str().unwrap();
                            let state = if thread == "first" { "idle" } else { status };
                            json!({"thread":{"id":thread, "status":{"type":state}}})
                        }
                        method => panic!("non-observation RPC: {method}"),
                    };
                    socket
                        .send(Message::Text(
                            json!({
                                "id": rpc["id"], "result": result
                            })
                            .to_string()
                            .into(),
                        ))
                        .await
                        .unwrap();
                }
                assert_eq!(round, 2);
            });
            let pid = if owner_matches { std::process::id() } else { 0 };
            assert_eq!(managed_daemon_is_idle(fixture.path(), pid).await, allowed);
            tokio::time::timeout(Duration::from_secs(1), rpc)
                .await
                .unwrap()
                .unwrap();
        }
    }
}
