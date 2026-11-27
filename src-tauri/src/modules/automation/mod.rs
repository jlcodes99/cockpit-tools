use super::automation_protocol::{
    AccountPlatform as Platform, Operation, Outcome, Reply, Request, VERSION,
};
use super::{codex_local_access, config, i18n, logger};
use crate::commands;
use serde_json::{json, Value};
use std::sync::LazyLock;
use tauri::{AppHandle, Emitter};
use tokio::sync::Mutex;

mod accounts;
mod quotas;

// A second switch on the same client could overwrite its credentials before the
// first client restart reads them. Both Antigravity targets can share credentials.
static SWITCHES: LazyLock<Vec<Mutex<()>>> =
    LazyLock::new(|| Platform::ALL.iter().map(|_| Mutex::new(())).collect());

#[derive(Debug)]
struct Failure {
    code: String,
    key: &'static str,
}

impl Failure {
    fn new(code: &str, key: &'static str) -> Self {
        Self {
            code: code.into(),
            key,
        }
    }

    fn host(error: String) -> Self {
        let prefix = error.split(':').next().unwrap_or_default();
        match prefix {
            "account_not_found" | "WORKBUDDY_AI_ACCOUNT_NOT_FOUND" => {
                Self::new("account_not_found", "common.automation.accountNotFound")
            }
            "APP_PATH_NOT_FOUND" => Self::new(prefix, "common.automation.appPathMissing"),
            "APP_PATH_MISSING_AFTER_SWITCH" => {
                Self::new(prefix, "common.automation.appPathMissingAfterSwitch")
            }
            _ if error.starts_with("API Key 账号不支持刷新配额") => {
                Self::new("quota_unsupported", "common.automation.quotaUnsupported")
            }
            _ => {
                // Provider errors may contain request URLs or credentials. Only
                // the host's structured error code crosses the automation boundary.
                let code = if !prefix.is_empty()
                    && prefix.len() <= 100
                    && prefix.contains('_')
                    && prefix
                        .bytes()
                        .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == b'_')
                {
                    prefix
                } else {
                    "operation_failed"
                };
                logger::log_warn(&format!("[Automation] {code}"));
                Self::new(code, "common.automation.operationFailed")
            }
        }
    }

    fn outcome(self) -> Outcome {
        let locale = config::get_user_config().language;
        Outcome::Error {
            code: self.code,
            message: i18n::translate(&locale, self.key, &[]),
        }
    }
}

async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, Failure> {
    tokio::task::spawn_blocking(work)
        .await
        .map_err(|error| Failure::host(error.to_string()))?
        .map_err(Failure::host)
}

fn check_id(id: &str) -> Result<(), Failure> {
    // Several legacy loaders derive a filename directly from an account ID.
    if id.is_empty()
        || id.trim() != id
        || id == "."
        || id == ".."
        || id.contains(['/', '\\', ':'])
        || id.chars().any(char::is_control)
    {
        return Err(Failure::new(
            "invalid_account_id",
            "common.automation.invalidAccountId",
        ));
    }
    Ok(())
}

fn check_ids(ids: &[String]) -> Result<(), Failure> {
    if ids.is_empty() {
        return Err(Failure::new(
            "invalid_account_id",
            "common.automation.invalidAccountId",
        ));
    }
    ids.iter().try_for_each(|id| check_id(id))
}

pub(super) async fn handle(request: Request, authorized: bool) -> Reply {
    let outcome = if !authorized {
        Failure::new("unauthorized", "common.automation.unauthorized").outcome()
    } else if request.version != VERSION {
        Failure::new("version_mismatch", "common.automation.versionMismatch").outcome()
    } else {
        match Box::pin(execute(request.command)).await {
            Ok(data) => Outcome::Ok { data },
            Err(error) => error.outcome(),
        }
    };
    Reply {
        version: VERSION,
        request_id: request.request_id,
        outcome,
    }
}

async fn execute(command: Operation) -> Result<Value, Failure> {
    match command {
        Operation::Platforms => Ok(json!(Platform::ALL
            .iter()
            .map(|p| p.as_str())
            .collect::<Vec<_>>())),
        Operation::List { platform } => Ok(json!(accounts::list(platform).await?)),
        Operation::Quota {
            platform,
            account_id,
            refresh,
        } => {
            check_id(&account_id)?;
            if refresh {
                accounts::lookup(platform, account_id.clone(), |_| ()).await?;
                Box::pin(quotas::refresh(app()?, platform, account_id.clone())).await?;
            }
            let snapshot = accounts::lookup(platform, account_id.clone(), quotas::snapshot).await?;
            if refresh && snapshot.refresh_failed() {
                return Err(Failure::new(
                    "quota_refresh_failed",
                    "common.automation.quotaRefreshFailed",
                ));
            }
            Ok(json!({ "platform": platform, "accountId": account_id, "quota": snapshot }))
        }
        Operation::Switch {
            platform,
            account_id,
        } => {
            check_id(&account_id)?;
            let group = match platform {
                Platform::AntigravityIde => Platform::Antigravity,
                _ => platform,
            };
            let index = Platform::ALL
                .iter()
                .position(|p| *p == group)
                .expect("registered platform");
            let _guard = SWITCHES[index]
                .try_lock()
                .map_err(|_| Failure::new("switch_busy", "common.automation.switchBusy"))?;
            accounts::lookup(platform, account_id.clone(), |_| ()).await?;
            let effect = Box::pin(switch(app()?, platform, account_id.clone())).await?;
            Ok(json!({
                "platform": platform, "accountId": account_id,
                "effect": effect.kind, "warnings": effect.warnings
            }))
        }
        Operation::ApiMembers => Ok(json!(codex_local_access::configured_service_members()
            .await
            .map_err(Failure::host)?)),
        Operation::ApiAdd { account_ids } => {
            check_ids(&account_ids)?;
            let result = Box::pin(codex_local_access::append_local_access_accounts(
                account_ids,
            ))
            .await
            .map_err(Failure::host)?;
            if !result.added_account_ids.is_empty() {
                let _ = app()?.emit("codex-local-access-state-updated", ());
            }
            Ok(json!({
                "members": result.state.service_members,
                "addedAccountIds": result.added_account_ids,
                "skippedAccounts": result.skipped_accounts
            }))
        }
        Operation::ApiRemove { account_ids } => {
            check_ids(&account_ids)?;
            let (state, removed_ids) = Box::pin(codex_local_access::remove_local_access_members(
                &account_ids,
            ))
            .await
            .map_err(Failure::host)?;
            if !removed_ids.is_empty() {
                let _ = app()?.emit("codex-local-access-state-updated", ());
            }
            Ok(json!({ "members": state.service_members, "removedAccountIds": removed_ids }))
        }
    }
}

fn app() -> Result<AppHandle, Failure> {
    crate::get_app_handle()
        .cloned()
        .ok_or_else(|| Failure::new("host_unavailable", "common.automation.hostUnavailable"))
}

struct SwitchEffect {
    kind: &'static str,
    warnings: Vec<&'static str>,
}

fn switch_effect(platform: Platform, message: &str) -> Result<SwitchEffect, Failure> {
    use Platform::*;
    let mut warnings = Vec::new();
    // These desktop commands return an on-screen summary after the credential
    // write. Never relay that text, which can contain paths and upstream errors.
    if message.starts_with("切换完成，但 ") {
        if message.contains("APP_PATH_NOT_FOUND:") && matches!(platform, Copilot | Kiro | Windsurf)
        {
            return Err(Failure::new(
                "APP_PATH_NOT_FOUND",
                "common.automation.appPathMissing",
            ));
        }
        warnings.push("client_launch_failed");
    }
    if matches!(platform, Trae | TraeSolo | TraeCn | TraeSoloCn)
        && (message.contains("；切号前刷新失败：") || message.contains("；Token 已过期，"))
    {
        warnings.push("credential_refresh_failed");
    }
    if platform == Claude && message.contains('（') && message.ends_with('）') {
        warnings.push("client_sync_warning");
    }
    let kind = if platform == Grok && message.starts_with("已准备独立目录: ") {
        "account_profile_prepared"
    } else {
        "client_credentials_applied"
    };
    Ok(SwitchEffect { kind, warnings })
}

async fn switch(app: AppHandle, platform: Platform, id: String) -> Result<SwitchEffect, Failure> {
    use Platform::*;
    let result = match platform {
        Antigravity => Box::pin(commands::account::switch_account(
            app,
            id,
            Some("antigravity".into()),
            Some(false),
            Some(true),
        ))
        .await
        .map(|_| String::new()),
        AntigravityIde => Box::pin(commands::account::switch_account(
            app,
            id,
            Some("antigravity_ide".into()),
            None,
            Some(true),
        ))
        .await
        .map(|_| String::new()),
        Codex => Box::pin(commands::codex::switch_codex_account(
            app,
            id,
            None,
            None,
            Some(false),
            Some(true),
        ))
        .await
        .map(|_| String::new()),
        Claude => Box::pin(commands::claude::switch_claude_account(app, id)).await,
        Zed => Box::pin(commands::zed::inject_zed_account(app, id)).await,
        Copilot => {
            Box::pin(commands::github_copilot::switch_github_copilot(
                app, id, true,
            ))
            .await
        }
        Windsurf => Box::pin(commands::windsurf::switch_windsurf(app, id, true)).await,
        Kiro => Box::pin(commands::kiro::switch_kiro(app, id, true)).await,
        Cursor => Box::pin(commands::cursor::inject_cursor_account(app, id)).await,
        Grok => {
            return switch_effect(
                platform,
                &blocking(move || commands::grok::switch_grok_account(app, id)).await?,
            )
        }
        Codebuddy => Box::pin(commands::codebuddy::inject_codebuddy_to_vscode(app, id, None)).await,
        CodebuddyCn => {
            Box::pin(commands::codebuddy_cn::inject_codebuddy_cn_to_vscode(
                app,
                id,
                Some(true),
                None,
            ))
            .await
        }
        Qoder => Box::pin(commands::qoder::inject_qoder_account(app, id)).await,
        QoderCn => Box::pin(commands::qoder_cn::inject_qoder_cn_account(app, id)).await,
        Zcode => {
            return switch_effect(
                platform,
                &blocking(move || commands::zcode::switch_zcode(app, id, true)).await?,
            )
        }
        Trae | TraeSolo | TraeCn | TraeSoloCn => {
            Box::pin(commands::trae::switch_trae(
                app,
                id,
                Some(platform.as_str().into()),
                true,
            ))
            .await
        }
        Workbuddy => Box::pin(commands::workbuddy::inject_workbuddy_to_vscode(app, id)).await,
        WorkbuddyAi => commands::workbuddy_ai::switch_workbuddy_ai_account(app, id)
            .await
            .map(|_| String::new()),
        Minimax => {
            Box::pin(commands::minimax_code::inject_minimax_code_to_client(
                app, id,
            ))
            .await
        }
    };
    switch_effect(platform, &result.map_err(Failure::host)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn automation_requires_authentication_and_exact_protocol_version() {
        let request = |version| Request {
            version,
            request_id: "fixture".into(),
            auth_token: String::new(),
            command: Operation::Platforms,
        };
        assert!(matches!(handle(request(VERSION), false).await.outcome,
            Outcome::Error { code, .. } if code == "unauthorized"));
        assert!(matches!(handle(request(VERSION + 1), true).await.outcome,
            Outcome::Error { code, .. } if code == "version_mismatch"));
        assert!(matches!(handle(request(VERSION), true).await.outcome,
            Outcome::Ok { data } if data.as_array().is_some_and(|ids| ids.len() == 21)));
    }

    #[test]
    fn automation_distinguishes_a_written_profile_from_a_launch_preflight_failure() {
        assert_eq!(
            switch_effect(Platform::Grok, "已准备独立目录: account (private path)")
                .unwrap()
                .kind,
            "account_profile_prepared"
        );
        assert_eq!(
            switch_effect(
                Platform::Workbuddy,
                "切换完成，但 WorkBuddy 启动失败：APP_PATH_NOT_FOUND:workbuddy"
            )
            .unwrap()
            .warnings,
            ["client_launch_failed"]
        );
        assert!(
            matches!(switch_effect(Platform::Kiro, "切换完成，但 Kiro 启动失败: APP_PATH_NOT_FOUND:kiro"),
            Err(Failure { code, .. }) if code == "APP_PATH_NOT_FOUND")
        );
    }
}
