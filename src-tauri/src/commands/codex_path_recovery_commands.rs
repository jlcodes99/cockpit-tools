/// Native menus and background switches do not pass through the frontend
/// recovery coordinator. Keep their exact operation, not a generic start,
/// when an early preflight/stop or late startup reports a missing path.
fn codex_path_recovery_payload(
    command: &str,
    args: serde_json::Value,
    managed: Option<bool>,
    error: &str,
) -> Option<serde_json::Value> {
    if managed == Some(true)
        || !matches!(
            command,
            "switch_codex_account" | "codex_local_access_activate"
        )
    {
        return None;
    }
    let error = error.trim();
    let missing = error == "APP_PATH_NOT_FOUND:codex"
        || error.strip_prefix("Codex API Service 已激活，但客户端启动失败: ")
            == Some("APP_PATH_NOT_FOUND:codex");
    missing.then(|| {
        serde_json::json!({
            "app": "codex",
            "retry": { "kind": "codexCommand", "command": command, "args": args }
        })
    })
}

#[tauri::command]
pub async fn switch_codex_account(
    app: AppHandle,
    account_id: String,
    auto_repair_mode: Option<codex_session_visibility::CodexSessionVisibilityAutoRepairMode>,
    reauth_token_generation: Option<u64>,
    launch_after_switch: Option<bool>,
    path_recovery_managed: Option<bool>,
) -> Result<CodexAccount, String> {
    let args = serde_json::json!({
        "accountId": account_id,
        "autoRepairMode": auto_repair_mode,
        "reauthTokenGeneration": reauth_token_generation,
        "launchAfterSwitch": launch_after_switch,
    });
    let result = Box::pin(switch_codex_account_internal(
        app.clone(),
        account_id,
        auto_repair_mode,
        reauth_token_generation,
        launch_after_switch,
    ))
    .await;
    if let Err(error) = &result {
        if let Some(payload) =
            codex_path_recovery_payload("switch_codex_account", args, path_recovery_managed, error)
        {
            let _ = app.emit("app:path_missing", payload);
        }
    }
    result
}

#[tauri::command]
pub async fn codex_local_access_activate(
    app: AppHandle,
    auto_repair_mode: Option<codex_session_visibility::CodexSessionVisibilityAutoRepairMode>,
    instance_id: Option<String>,
    launch_after_switch: Option<bool>,
    path_recovery_managed: Option<bool>,
) -> Result<CodexLocalAccessState, String> {
    let args = serde_json::json!({
        "autoRepairMode": auto_repair_mode,
        "instanceId": instance_id,
        "launchAfterSwitch": launch_after_switch,
    });
    let result = Box::pin(codex_local_access_activate_internal(
        app.clone(),
        auto_repair_mode,
        instance_id,
        launch_after_switch,
    ))
    .await;
    if let Err(error) = &result {
        if let Some(payload) = codex_path_recovery_payload(
            "codex_local_access_activate",
            args,
            path_recovery_managed,
            error,
        ) {
            let _ = app.emit("app:path_missing", payload);
        }
    }
    result
}

#[cfg(test)]
mod path_recovery_command_tests {
    use super::*;

    #[test]
    fn native_switch_recovery_preserves_account_generation_and_no_launch_intent() {
        let args = serde_json::json!({
            "accountId":"selected-account", "autoRepairMode":codex_session_visibility::CodexSessionVisibilityAutoRepairMode::Current,
            "reauthTokenGeneration":41, "launchAfterSwitch":false,
        });
        let payload = codex_path_recovery_payload(
            "switch_codex_account",
            args.clone(),
            None,
            "APP_PATH_NOT_FOUND:codex",
        )
        .unwrap();
        assert_eq!(payload["retry"]["command"], "switch_codex_account");
        assert_eq!(payload["retry"]["kind"], "codexCommand");
        assert_eq!(payload["retry"]["args"], args);
        assert_eq!(payload["app"], "codex");
    }

    #[test]
    fn native_api_recovery_keeps_selected_instance_for_early_and_late_errors() {
        let args = serde_json::json!({"instanceId":"managed-profile", "autoRepairMode":null, "launchAfterSwitch":true});
        for error in [
            "APP_PATH_NOT_FOUND:codex",
            "Codex API Service 已激活，但客户端启动失败: APP_PATH_NOT_FOUND:codex",
        ] {
            let payload = codex_path_recovery_payload(
                "codex_local_access_activate",
                args.clone(),
                Some(false),
                error,
            )
            .unwrap();
            assert_eq!(payload["retry"]["args"], args);
        }
    }

    #[test]
    fn managed_recovery_and_unrelated_failures_do_not_broadcast_another_dialog() {
        let args = serde_json::json!({"instanceId":"managed-profile"});
        assert!(codex_path_recovery_payload(
            "codex_local_access_activate",
            args.clone(),
            Some(true),
            "APP_PATH_NOT_FOUND:codex"
        )
        .is_none());
        for error in [
            "APP_PATH_NOT_FOUND:claude",
            "CODEX_SWITCH_AUTH_REQUIRED:test",
            "access denied",
            "APP_PATH_NOT_FOUND:codex trailing error",
        ] {
            assert!(
                codex_path_recovery_payload("switch_codex_account", args.clone(), None, error)
                    .is_none()
            );
        }
        assert!(codex_path_recovery_payload(
            "unrelated_command",
            args,
            None,
            "APP_PATH_NOT_FOUND:codex"
        )
        .is_none());
    }
}
