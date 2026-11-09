//! One-shot SSH reconnect over the default profile's existing loopback channel.
//! No new-document hooks: reconnect happens only after a verified auth write.
use super::*;
use crate::models::ssh_server::SshServer;

#[path = "ssh_server_identity.rs"]
mod identity;

#[path = "codex_app_ssh_transport.rs"]
mod transport;

const SCRIPT: &str = include_str!("codex_app_ssh_reconnect.js");
const REFRESH_TIMEOUT: Duration = Duration::from_secs(40);

fn current(account: &CodexAccount) -> bool {
    codex_account::get_current_account().is_some_and(|active| {
        active.id == account.id && active.token_generation == account.token_generation
    })
}

async fn resolve_endpoint(
    host: &str,
    explicit: Option<&SshServer>,
) -> Result<identity::Endpoint, String> {
    let mut command = tokio::process::Command::new("ssh");
    command.args(["-G", "-o", "CanonicalizeHostname=no"]);
    if let Some(server) = explicit {
        command.args(["-l", &server.username, "-p", &server.port.to_string()]);
    }
    command.args(["--", host]);
    #[cfg(target_os = "windows")]
    command.creation_flags(0x0800_0000);
    let output =
        crate::modules::ssh_server::command_output(command, Duration::from_secs(3), None).await?;
    if !output.status.success() {
        return Err("ssh_config_unavailable".into());
    }
    identity::endpoint(&String::from_utf8_lossy(&output.stdout))
        .ok_or_else(|| "ssh_config_invalid".into())
}

async fn matching_host(profile: &Path, server: &SshServer) -> Result<Option<String>, String> {
    let raw = fs::read(profile.join(".codex-global-state.json"))
        .map_err(|_| "profile_state_unavailable")?;
    let state: Value = serde_json::from_slice(&raw).map_err(|_| "profile_state_invalid")?;
    let target = resolve_endpoint(&server.host, Some(server)).await?;
    let mut matches = Vec::new();
    for (host, alias) in identity::aliases(&state) {
        if resolve_endpoint(&alias, None)
            .await
            .as_ref()
            .is_ok_and(|endpoint| *endpoint == target)
        {
            matches.push(host);
        }
    }
    match matches.len() {
        0 => Ok(None),
        1 => Ok(matches.pop()),
        _ => Err("ssh_connection_ambiguous".into()),
    }
}

enum DesktopAvailability {
    Absent,
    Starting,
    Ready(u16),
}

fn ensure_current_target(account: &CodexAccount) -> Result<(), String> {
    if !current(account) {
        return Err("ssh_sync_superseded".into());
    }
    let settings = crate::modules::codex_instance::load_default_settings()?;
    if bind_account_id_value(settings.bind_account_id.as_deref()).as_deref()
        != Some(account.id.as_str())
    {
        return Err("profile_binding_changed".into());
    }
    Ok(())
}

async fn desktop_availability(
    account: &CodexAccount,
    wait: bool,
) -> Result<DesktopAvailability, String> {
    let settings = crate::modules::codex_instance::load_default_settings()?;
    if settings.launch_mode != crate::models::InstanceLaunchMode::App {
        return Ok(DesktopAvailability::Absent);
    }
    let (pid, port) = tokio::task::spawn_blocking(move || {
        let entries = crate::modules::process::collect_codex_process_entries();
        let pid = crate::modules::process::resolve_codex_pid_from_entries(
            settings.last_pid,
            None,
            &entries,
        );
        (pid, pid.and_then(remote_debugging_port_for_pid))
    })
    .await
    .map_err(|_| "process_snapshot_unavailable")?;
    if pid.is_none() && !wait {
        return Ok(DesktopAvailability::Absent);
    }
    if pid.is_some() {
        ensure_current_target(account)?;
    }
    Ok(port
        .map(DesktopAvailability::Ready)
        .unwrap_or(DesktopAvailability::Starting))
}

fn request_script(host: &str, account: &CodexAccount, request_id: &str) -> String {
    let oauth = !account.is_api_key_auth()
        && !account.is_agent_identity_auth()
        && !account.is_web_session_auth();
    let auth_kind = if account.is_api_key_auth() {
        Some("apiKey")
    } else if oauth {
        Some("chatgpt")
    } else {
        None
    };
    let request = json!({"hostId":host,"requestId":request_id,
        "email":oauth.then_some(&account.email),"authKind":auth_kind,
        "accountId":account.account_id,"timeoutMs":30000});
    SCRIPT.replace("__COCKPIT_SSH_RECONNECT__", &request.to_string())
}

async fn try_refresh(
    client: &Client,
    port: u16,
    profile: &Path,
    server: &SshServer,
    account: &CodexAccount,
    request_id: &str,
) -> Result<bool, String> {
    let targets = query_targets(client, port).await;
    if !targets.iter().any(is_codex_app_target) {
        return Ok(false);
    }
    let Some(host) = matching_host(profile, server).await? else {
        return Ok(false);
    };
    let script = request_script(&host, account, request_id);
    for target in targets.iter().filter(|target| is_codex_app_target(target)) {
        ensure_current_target(account)?;
        let Some(url) = target.websocket_url.as_deref() else {
            continue;
        };
        let cancelled = async {
            while ensure_current_target(account).is_ok() {
                tokio::time::sleep(Duration::from_millis(250)).await;
            }
        };
        let value = match transport::evaluate(url, &script, cancelled).await {
            Ok(value) => value,
            // Startup overlays and old renderers can disappear during a switch.
            // Re-query the native pages and reuse the same request identity.
            Err(reason) if transport::retryable(&reason) => continue,
            Err(reason) => return Err(reason),
        };
        if value["ok"] == true {
            return Ok(true);
        }
        let reason = value["reason"]
            .as_str()
            .unwrap_or("bridge_invalid_response");
        if !matches!(reason, "bridge_unavailable" | "not_ready") {
            if let Some(error) = value.get("readError") {
                crate::modules::logger::log_warn(&format!(
                    "[Codex SSH] 原生账号核验失败: code={}, message={}",
                    error["code"], error["message"]
                ));
            }
            return Err(reason.to_string());
        }
    }
    Ok(false)
}

pub(super) async fn refresh(
    server: &SshServer,
    account: &CodexAccount,
    wait_for_desktop: bool,
) -> Result<bool, String> {
    timeout(REFRESH_TIMEOUT, async {
        let profile = crate::modules::codex_instance::get_default_codex_home()?;
        let client = Client::builder()
            .no_proxy()
            .build()
            .map_err(|_| "bridge_unavailable")?;
        let request_id = uuid::Uuid::new_v4().to_string();
        loop {
            if !current(account) {
                return Err("ssh_sync_superseded".into());
            }
            match desktop_availability(account, wait_for_desktop).await? {
                DesktopAvailability::Absent => return Ok(false),
                DesktopAvailability::Starting => {}
                DesktopAvailability::Ready(port) => {
                    if try_refresh(&client, port, &profile, server, account, &request_id).await? {
                        return Ok(true);
                    }
                }
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
    })
    .await
    .map_err(|_| "timeout".to_string())?
}
