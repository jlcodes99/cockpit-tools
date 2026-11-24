use super::{resolve_ws_switch_target, WsMessage, WsSwitchTarget};

#[test]
fn legacy_switch_payload_keeps_ide_target_and_omits_new_fields() {
    let message: WsMessage = serde_json::from_str(
        r#"{"type":"request.switch_account","payload":{"account_id":"account-a","request_id":"x"}}"#,
    ).unwrap();
    let WsMessage::SwitchAccount {
        runtime_target,
        launch_on_switch,
        ..
    } = &message
    else {
        panic!("expected switch request");
    };
    assert_eq!(
        resolve_ws_switch_target(runtime_target.as_deref(), *launch_on_switch).unwrap(),
        WsSwitchTarget::Ide
    );
    let json = serde_json::to_value(message).unwrap();
    assert!(json["payload"].get("runtime_target").is_none());
    assert!(json["payload"].get("launch_on_switch").is_none());
}

#[test]
fn desktop_switch_requires_opt_in_launch() {
    for launch in [None, Some(false)] {
        assert_eq!(
            resolve_ws_switch_target(Some("antigravity"), launch).unwrap(),
            WsSwitchTarget::Desktop {
                launch_on_switch: false
            }
        );
    }
    assert_eq!(
        resolve_ws_switch_target(Some(" ANTIGRAVITY "), Some(true)).unwrap(),
        WsSwitchTarget::Desktop {
            launch_on_switch: true
        }
    );
}

#[test]
fn explicit_ide_target_keeps_existing_behavior_and_rejects_desktop_launch_flag() {
    assert_eq!(
        resolve_ws_switch_target(Some("antigravity_ide"), None).unwrap(),
        WsSwitchTarget::Ide
    );
    assert_eq!(
        resolve_ws_switch_target(Some("antigravity_ide"), Some(false)).unwrap_err(),
        "WS_SWITCH_LAUNCH_POLICY_INVALID"
    );
    assert_eq!(
        resolve_ws_switch_target(None, Some(true)).unwrap_err(),
        "WS_SWITCH_LAUNCH_POLICY_INVALID"
    );
    assert_eq!(
        resolve_ws_switch_target(Some("unsupported"), None).unwrap_err(),
        "WS_SWITCH_TARGET_INVALID"
    );
}

#[test]
fn desktop_payload_deserializes_explicit_no_launch() {
    let message: WsMessage = serde_json::from_str(
        r#"{"type":"request.switch_account","payload":{"account_id":"account-a","runtime_target":"antigravity","launch_on_switch":false}}"#,
    ).unwrap();
    let WsMessage::SwitchAccount {
        runtime_target,
        launch_on_switch,
        ..
    } = message
    else {
        panic!("expected switch request");
    };
    assert_eq!(
        resolve_ws_switch_target(runtime_target.as_deref(), launch_on_switch).unwrap(),
        WsSwitchTarget::Desktop {
            launch_on_switch: false
        }
    );
}

#[tokio::test]
async fn automation_websocket_authenticates_and_replies_only_to_requester() {
    use super::{handle_connection, ws_auth_token, WsServer};
    use crate::modules::automation_protocol::{Operation, Outcome, Request, VERSION};
    use futures_util::{SinkExt, StreamExt};
    use std::{sync::Arc, time::Duration};
    use tokio::net::TcpListener;
    use tokio_tungstenite::{connect_async, tungstenite::Message};

    tokio::time::timeout(Duration::from_secs(5), async {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = Arc::new(WsServer::new());
        let mut broadcasts = server.tx.subscribe();
        let worker = tokio::spawn(async move {
            let (stream, peer) = listener.accept().await.unwrap();
            Box::pin(handle_connection(server, stream, peer)).await;
        });

        let (mut client, _) = connect_async(format!("ws://{addr}")).await.unwrap();
        let ready = client.next().await.unwrap().unwrap().into_text().unwrap();
        assert!(matches!(serde_json::from_str(&ready).unwrap(), WsMessage::Ready { .. }));
        for (id, token) in [("invalid", "fixture-invalid"), ("valid", ws_auth_token())] {
            let command = WsMessage::Automation(Request {
                version: VERSION,
                request_id: id.into(),
                auth_token: token.into(),
                command: Operation::Platforms,
            });
            client.send(Message::Text(serde_json::to_string(&command).unwrap().into())).await.unwrap();
            let reply = client.next().await.unwrap().unwrap().into_text().unwrap();
            let WsMessage::AutomationReply(reply) = serde_json::from_str(&reply).unwrap() else {
                panic!("expected a direct automation reply");
            };
            assert_eq!(reply.request_id, id);
            match id {
                "invalid" => assert!(matches!(reply.outcome, Outcome::Error { code, .. } if code == "unauthorized")),
                _ => assert!(matches!(reply.outcome, Outcome::Ok { .. })),
            }
        }
        assert!(matches!(broadcasts.try_recv(), Err(tokio::sync::broadcast::error::TryRecvError::Empty)));
        drop(client);
        worker.await.unwrap();
    }).await.expect("automation exchange did not finish");
}
