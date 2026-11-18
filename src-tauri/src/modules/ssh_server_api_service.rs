//! Keep one reverse SSH forward per synchronized Codex profile.
use super::*;
use crate::modules::codex_local_access::SshApiService;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, MutexGuard};
use tauri::Emitter;
use tokio::sync::Notify;

const RETRY_INTERVAL: Duration = Duration::from_secs(20);
static CHANGED: Notify = Notify::const_new();
static WAIT_FOR_DESKTOP: AtomicBool = AtomicBool::new(false);
static TUNNELS: LazyLock<Mutex<Tunnels>> = LazyLock::new(Mutex::default);

#[derive(Default)]
struct Tunnels {
    generation: u64,
    by_server: HashMap<String, Tunnel>,
}

struct Tunnel {
    server: SshServer,
    local_port: u16,
    forward: process::ReverseForward,
    applied: Option<SshApiService>,
    automatic: bool,
}

fn tunnels() -> MutexGuard<'static, Tunnels> {
    TUNNELS.lock().unwrap_or_else(|error| error.into_inner())
}

pub(crate) fn close_api_service_tunnels() {
    let mut tunnels = tunnels();
    tunnels.generation += 1;
    tunnels.by_server.clear();
}

pub(crate) fn api_service_activated(wait_for_desktop: bool) {
    if wait_for_desktop {
        WAIT_FOR_DESKTOP.store(true, Ordering::Release);
    }
    CHANGED.notify_one();
}

pub(crate) fn default_binding_saved(binding: Option<&str>) {
    if !binding.is_some_and(codex_instance::is_api_service_bind_account_id) {
        close_api_service_tunnels();
    }
    CHANGED.notify_one();
}

pub(super) fn is_bound() -> bool {
    !crate::modules::app_lifecycle::is_shutdown_started()
        && codex_instance::load_default_settings().is_ok_and(|settings| {
            settings
                .bind_account_id
                .as_deref()
                .is_some_and(codex_instance::is_api_service_bind_account_id)
        })
}

fn same_target(saved: &SshServer, current: &SshServer) -> bool {
    saved.id == current.id
        && saved.host == current.host
        && saved.port == current.port
        && saved.username == current.username
        && saved.auth == current.auth
        && saved.codex_home == current.codex_home
}

fn is_selected(server: &SshServer, selected_ids: &[String]) -> bool {
    server.sync_on_codex_switch && selected_ids.contains(&server.id)
}

pub(super) fn server_list_changed(list: &SshServerList) {
    tunnels().by_server.retain(|id, tunnel| {
        list.servers
            .iter()
            .find(|server| &server.id == id)
            .is_some_and(|server| {
                same_target(&tunnel.server, server)
                    && (!tunnel.automatic || is_selected(server, &list.selected_server_ids))
            })
    });
    CHANGED.notify_one();
}

pub(super) struct SyncRoute<'a> {
    service: &'a SshApiService,
    automatic: bool,
    generation: u64,
}

impl SyncRoute<'_> {
    pub(super) async fn verify(&self, server: &SshServer) -> Result<(), String> {
        let store = load_store()?;
        let still_selected = store.servers.iter().any(|candidate| {
            same_target(server, candidate)
                && (!self.automatic || is_selected(candidate, &store.selected_server_ids))
        });
        if still_selected
            && is_bound()
            && codex_local_access::running_api_service_for_ssh()
                .await
                .as_ref()
                == Some(self.service)
            && tunnels().generation == self.generation
        {
            return Ok(());
        }
        Err("ssh_sync_superseded: API Service or SSH target changed".into())
    }

    async fn connect(&self, server: &SshServer) -> Result<u16, String> {
        self.verify(server).await?;
        validate_server(server)?;
        if let Some(port) = live_port(server, self.service.port) {
            return Ok(port);
        }
        let forwarding = format!("127.0.0.1:0:127.0.0.1:{}", self.service.port);
        let mut command = ssh_command(
            server,
            TEST_COMMAND_TIMEOUT_SECS,
            &[
                "-o",
                "IgnoreUnknown=ForkAfterAuthentication,SessionType,StdinNull",
                "-T",
                "-o",
                "ExitOnForwardFailure=yes",
                "-o",
                "LogLevel=INFO",
                "-o",
                "ControlMaster=no",
                "-o",
                "ControlPath=none",
                "-o",
                "ForkAfterAuthentication=no",
                "-o",
                "RemoteCommand=none",
                "-o",
                "SessionType=default",
                "-o",
                "StdinNull=no",
                "-o",
                "ForwardAgent=no",
                "-o",
                "PermitLocalCommand=no",
                "-o",
                "ClearAllForwardings=no",
                "-R",
                &forwarding,
            ],
        );
        command.args(["exec", "cat", ">/dev/null"]);
        let forward = process::reverse_forward(
            command,
            self.service.port,
            Duration::from_secs(TEST_COMMAND_TIMEOUT_SECS),
        )
        .await
        .map_err(|error| format!("ssh_tunnel_failed: {}", sanitize_error(error)))?;
        self.verify(server).await?;
        let port = forward.remote_port();
        let mut tunnels = tunnels();
        if tunnels.generation != self.generation {
            return Err("ssh_sync_superseded: API Service stopped".into());
        }
        tunnels.by_server.insert(
            server.id.clone(),
            Tunnel {
                server: server.clone(),
                local_port: self.service.port,
                forward,
                applied: None,
                automatic: self.automatic,
            },
        );
        Ok(port)
    }

    pub(super) async fn verify_forward(&self, server: &SshServer) -> Result<(), String> {
        self.verify(server).await?;
        live_port(server, self.service.port)
            .map(|_| ())
            .ok_or_else(|| "ssh_tunnel_failed: reverse forwarding disconnected".into())
    }
}

fn live_port(server: &SshServer, local_port: u16) -> Option<u16> {
    let mut tunnels = tunnels();
    let tunnel = tunnels.by_server.get_mut(&server.id)?;
    (same_target(server, &tunnel.server)
        && tunnel.local_port == local_port
        && tunnel.forward.is_running())
    .then(|| tunnel.forward.remote_port())
}

pub(super) async fn sync_server(
    server: SshServer,
    service: &SshApiService,
    automatic: bool,
    wait_for_desktop: bool,
) -> SshCodexSyncResult {
    let generation = tunnels().generation;
    let route = SyncRoute {
        service,
        automatic,
        generation,
    };
    let port = match route.connect(&server).await {
        Ok(port) => port,
        Err(error) => {
            let status = SshCodexSyncStatus {
                account_id: codex_instance::CODEX_API_SERVICE_BIND_ACCOUNT_ID.into(),
                account_email: "Codex API Service".into(),
                token_generation: 0,
                bundle_hash: String::new(),
                synced_at: now_timestamp(),
                verified: false,
                error: Some(error),
            };
            return if route.verify(&server).await.is_ok() {
                publish_sync_status(&server, status)
            } else {
                result_from_status(&server, status)
            };
        }
    };
    let account = service.remote_account(port);
    let result =
        sync_account_to_server(server.clone(), &account, wait_for_desktop, Some(&route)).await;
    let mut tunnels = tunnels();
    if let Some(tunnel) = tunnels.by_server.get_mut(&server.id) {
        if result.verified {
            tunnel.applied = result.succeeded().then(|| service.clone());
            tunnel.automatic = automatic;
        } else if tunnel.applied.is_none() {
            tunnels.by_server.remove(&server.id);
        }
    }
    result
}

fn publish_result(result: SshCodexSyncResult) {
    if let Some(error) = &result.error {
        if error.starts_with("ssh_sync_superseded:") {
            return;
        }
        logger::log_warn(&format!(
            "[Codex SSH] API 服务同步失败: server_id={}, error={}",
            result.server_id, error,
        ));
    }
    if let Some(app) = crate::get_app_handle() {
        let _ = app.emit("codex:ssh-sync-result", &result);
    }
}

async fn reconcile(wait_for_desktop: bool) -> bool {
    if !is_bound() {
        close_api_service_tunnels();
        return false;
    }
    let Some(service) = codex_local_access::running_api_service_for_ssh().await else {
        close_api_service_tunnels();
        return true;
    };
    let Ok(store) = load_store() else { return true };
    let manual_ids: Vec<String> = tunnels()
        .by_server
        .iter()
        .filter(|(_, tunnel)| !tunnel.automatic)
        .map(|(id, _)| id.clone())
        .collect();
    for server in store.servers {
        let automatic = is_selected(&server, &store.selected_server_ids);
        if !automatic && !manual_ids.contains(&server.id) {
            continue;
        }
        let _guard = ACCOUNT_SYNC_LOCK.lock().await;
        let synced = tunnels()
            .by_server
            .get_mut(&server.id)
            .is_some_and(|tunnel| {
                same_target(&server, &tunnel.server)
                    && tunnel.applied.as_ref() == Some(&service)
                    && tunnel.forward.is_running()
            });
        // Activation may launch Desktop after a background pass already wrote the files.
        if synced && !wait_for_desktop {
            continue;
        }
        publish_result(sync_server(server, &service, automatic, wait_for_desktop).await);
    }
    true
}

pub(crate) async fn run_api_service_sync() {
    while !crate::modules::app_lifecycle::is_shutdown_started() {
        let wait = WAIT_FOR_DESKTOP.swap(false, Ordering::AcqRel);
        if reconcile(wait).await {
            tokio::select! {
                _ = CHANGED.notified() => {}
                _ = tokio::time::sleep(RETRY_INTERVAL) => {}
            }
        } else {
            CHANGED.notified().await;
        }
    }
}
