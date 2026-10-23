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
