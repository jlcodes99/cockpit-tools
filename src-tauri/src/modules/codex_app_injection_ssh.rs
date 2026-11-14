//! One-shot SSH reconnect over the default profile's existing loopback channel.
//! No new-document hooks: reconnect happens only after a verified auth write.
use super::*;
use crate::models::ssh_server::SshServer;
use crate::modules::ssh_server_identity as identity;

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

async fn matching_host(state: &Value, server: &SshServer) -> Result<Option<String>, String> {
    let aliases = identity::aliases(state);
    if aliases.is_empty() {
        return Ok(None);
    }
    let target = resolve_endpoint(&server.host, Some(server)).await?;
    let mut matches = Vec::new();
    for (host, alias) in aliases {
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

enum RefreshAttempt {
    Confirmed,
    NoEnabledConnection,
    Waiting(&'static str),
}

fn pending_preferences(state: &Value, expected_owner: Option<&str>) -> Option<&'static str> {
    match identity::renderer_owner(state) {
        Some(owner) if expected_owner.is_some_and(|expected| owner != expected) => {
            Some("desktop_account")
        }
        // Older clients can clear the legacy switches while restoring account state.
        None if identity::auto_connect(state).is_null()
            && state[identity::CONNECTIONS]
                .as_array()
                .is_some_and(|hosts| !hosts.is_empty()) =>
        {
            Some("native_ssh_preferences")
        }
        _ => None,
    }
}

fn native_refresh_result(value: Value) -> Result<bool, String> {
    if value["ok"] == true {
        return Ok(true);
    }
    let reason = value["reason"]
        .as_str()
        .unwrap_or("bridge_invalid_response");
    if matches!(reason, "bridge_unavailable" | "not_ready") {
        return Ok(false);
    }
    if let Some(error) = value.get("readError") {
        crate::modules::logger::log_warn(&format!(
            "[Codex SSH] 原生账号核验失败: code={}, message={}",
            error["code"], error["message"]
        ));
    }
    Err(reason.to_string())
}

async fn refresh_page(url: &str, script: &str, account: &CodexAccount) -> Result<bool, String> {
    ensure_current_target(account)?;
    let cancelled = async {
        while ensure_current_target(account).is_ok() {
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
    };
    match transport::evaluate(url, script, cancelled).await {
        Ok(value) => native_refresh_result(value),
        // Startup overlays can close while the main renderer is being replaced.
        Err(reason) if transport::retryable(&reason) => Ok(false),
        Err(reason) => Err(reason),
    }
}

async fn try_refresh(
    client: &Client,
    port: u16,
    profile: &Path,
    server: &SshServer,
    account: &CodexAccount,
    request_id: &str,
    expected_owner: Option<&str>,
) -> Result<RefreshAttempt, String> {
    let pages: Vec<_> = query_targets(client, port)
        .await
        .into_iter()
        .filter(is_codex_app_target)
        .filter_map(|target| target.websocket_url)
        .collect();
    if pages.is_empty() {
        return Ok(RefreshAttempt::Waiting("desktop_page"));
    }
    let raw = fs::read(profile.join(".codex-global-state.json"))
        .map_err(|_| "profile_state_unavailable")?;
    let state = serde_json::from_slice(&raw).map_err(|_| "profile_state_invalid")?;
    if let Some(stage) = pending_preferences(&state, expected_owner) {
        return Ok(RefreshAttempt::Waiting(stage));
    }
    let Some(host) = matching_host(&state, server).await? else {
        return Ok(RefreshAttempt::NoEnabledConnection);
    };
    let script = request_script(&host, account, request_id);
    for url in pages {
        if refresh_page(&url, &script, account).await? {
            return Ok(RefreshAttempt::Confirmed);
        }
    }
    Ok(RefreshAttempt::Waiting("native_ssh_connection"))
}

async fn projected_owner(profile: &Path) -> Option<String> {
    let profile = profile.to_path_buf();
    tokio::task::spawn_blocking(move || codex_account::desktop_renderer_project_owner(&profile))
        .await
        .ok()
        .and_then(Result::ok)
        .flatten()
        .map(|owner| owner.unwrap_or_else(|| identity::SIGNED_OUT_OWNER.to_string()))
}

pub(super) async fn refresh(
    server: &SshServer,
    account: &CodexAccount,
    wait_for_desktop: bool,
) -> Result<bool, String> {
    let mut waiting_on = "desktop_startup";
    let result = timeout(REFRESH_TIMEOUT, async {
        let profile = crate::modules::codex_instance::get_default_codex_home()?;
        let expected_owner = projected_owner(&profile).await;
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
                DesktopAvailability::Starting => waiting_on = "desktop_startup",
                DesktopAvailability::Ready(port) => {
                    waiting_on = "native_ssh_refresh";
                    match try_refresh(
                        &client,
                        port,
                        &profile,
                        server,
                        account,
                        &request_id,
                        expected_owner.as_deref(),
                    )
                    .await?
                    {
                        RefreshAttempt::Confirmed => return Ok(true),
                        RefreshAttempt::NoEnabledConnection => return Ok(false),
                        RefreshAttempt::Waiting(stage) => waiting_on = stage,
                    }
                }
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
    })
    .await;
    result.map_err(|_| format!("{waiting_on}_timeout"))?
}
