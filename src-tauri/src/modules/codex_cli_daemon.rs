//! Notify users after profile changes and restart the shared CLI daemon on request.

use std::path::Path;
#[cfg(any(unix, windows))]
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{LazyLock, Mutex},
    time::Duration,
};

#[cfg(unix)]
const CONTROL_SOCKET: &str = "app-server-control/app-server-control.sock";

#[cfg(unix)]
#[path = "codex_cli_daemon_activity.rs"]
mod activity;
#[cfg(unix)]
pub(crate) use activity::managed_daemon_is_idle;

#[cfg(any(unix, windows))]
const RESTART_UNCONFIRMED: &str = "CODEX_CLI_DAEMON_RESTART_UNCONFIRMED";
#[cfg(unix)]
const WORKSPACE_MISMATCH: &str = "CODEX_CLI_DAEMON_WORKSPACE_MISMATCH";

#[cfg(any(unix, windows))]
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum RestartOutcome {
    Restarted,
    #[cfg(unix)]
    WorkspaceConfirmed,
    WorkspaceUnconfirmed,
}

#[cfg(unix)]
async fn connect_control_socket(codex_home: &Path) -> std::io::Result<tokio::net::UnixStream> {
    let socket = codex_home.join(CONTROL_SOCKET);
    // Long profile paths can point at a short runtime socket.
    let socket = if std::os::unix::net::SocketAddr::from_pathname(&socket).is_err() {
        tokio::fs::canonicalize(socket).await?
    } else {
        socket
    };
    tokio::net::UnixStream::connect(socket).await
}

#[cfg(unix)]
async fn daemon_is_listening(codex_home: &Path) -> bool {
    let live = tokio::time::timeout(
        Duration::from_millis(250),
        connect_control_socket(codex_home),
    )
    .await;
    matches!(live, Ok(Ok(_)))
}

#[cfg(windows)]
async fn windows_control_socket_path(codex_home: &Path) -> Option<PathBuf> {
    let control_dir = tokio::time::timeout(
        Duration::from_millis(250),
        tokio::fs::canonicalize(codex_home.join("app-server-control")),
    )
    .await
    .ok()?
    .ok()?;
    Some(control_dir.join("app-server-control.sock"))
}

#[cfg(windows)]
fn connect_windows_control_socket(path: &Path) -> Option<socket2::Socket> {
    use socket2::{Domain, SockAddr, Socket, Type};

    let path = crate::modules::process::normalize_windows_user_facing_path(path.to_str()?);
    let address = SockAddr::unix(path).ok()?;
    let socket = Socket::new(Domain::UNIX, Type::STREAM, None).ok()?;
    socket
        .connect_timeout(&address, Duration::from_millis(250))
        .ok()?;
    Some(socket)
}

#[cfg(windows)]
async fn daemon_is_listening(codex_home: &Path) -> bool {
    let Some(path) = windows_control_socket_path(codex_home).await else {
        return false;
    };
    tokio::task::spawn_blocking(move || connect_windows_control_socket(&path).is_some())
        .await
        .unwrap_or(false)
}

#[cfg(windows)]
async fn daemon_peer_pid(codex_home: &Path) -> Option<u32> {
    use std::os::windows::io::AsRawSocket;
    use windows::Win32::Networking::WinSock::{
        WSAIoctl, SIO_AF_UNIX_GETPEERPID, SOCKET, SOCKET_ERROR,
    };
    let path = windows_control_socket_path(codex_home).await?;
    tokio::task::spawn_blocking(move || {
        let socket = connect_windows_control_socket(&path)?;
        let mut pid = 0u32;
        let mut returned = 0;
        // The IOCTL returns the kernel's peer PID for this connected AF_UNIX socket.
        let result = unsafe {
            WSAIoctl(
                SOCKET(socket.as_raw_socket() as usize),
                SIO_AF_UNIX_GETPEERPID,
                None,
                0,
                Some(std::ptr::addr_of_mut!(pid).cast()),
                std::mem::size_of::<u32>() as u32,
                &mut returned,
                None,
                None,
            )
        };
        (result != SOCKET_ERROR && pid != 0).then_some(pid)
    })
    .await
    .ok()
    .flatten()
}

#[cfg(unix)]
async fn daemon_peer_pid(codex_home: &Path) -> Option<u32> {
    let socket = tokio::time::timeout(
        Duration::from_millis(250),
        connect_control_socket(codex_home),
    )
    .await
    .ok()?
    .ok()?;
    u32::try_from(socket.peer_cred().ok()?.pid()?)
        .ok()
        .filter(|pid| *pid != 0)
}

#[cfg(any(unix, windows))]
async fn restart_notice(codex_home: &Path) -> Option<String> {
    if !daemon_is_listening(codex_home).await {
        return None;
    }
    // The terminal may have a different working directory. Shell quoting is only
    // for the displayed command; Cockpit never executes this command.
    let home = std::path::absolute(codex_home).ok()?;
    let home = home.to_str()?;
    #[cfg(unix)]
    let command = format!(
        "CODEX_HOME='{}' codex app-server daemon restart",
        home.replace('\'', "'\"'\"'")
    );
    #[cfg(windows)]
    let command = {
        let escaped = home
            .replace('\'', "''")
            .replace('‘', "‘‘")
            .replace('’', "’’");
        format!("& {{ $previousHome = $env:CODEX_HOME; try {{ $env:CODEX_HOME = '{escaped}'; codex app-server daemon restart }} finally {{ $env:CODEX_HOME = $previousHome }} }}")
    };
    Some(command)
}

#[cfg(any(unix, windows))]
#[derive(Clone)]
struct ProfileWrite {
    complete: bool,
    gateway_account_id: Option<String>,
}

#[cfg(any(unix, windows))]
#[derive(Default)]
struct RestartNotices {
    pending: HashMap<PathBuf, (uuid::Uuid, ProfileWrite)>,
    restartable: HashMap<PathBuf, Option<String>>,
    #[cfg(target_os = "macos")]
    switching: HashMap<PathBuf, Option<ProfileWrite>>,
}

#[cfg(any(unix, windows))]
impl RestartNotices {
    fn begin_probe(&mut self, home: PathBuf, write: ProfileWrite) -> Option<uuid::Uuid> {
        self.restartable.remove(&home);
        if let Some((_, pending)) = self.pending.get_mut(&home) {
            *pending = write;
            return None;
        }
        let probe = uuid::Uuid::new_v4();
        self.pending.insert(home, (probe, write));
        Some(probe)
    }

    fn finish_probe(&mut self, home: &Path, probe: uuid::Uuid, live: bool) -> Option<bool> {
        if self.pending.get(home)?.0 != probe {
            return None;
        }
        let (_, write) = self.pending.remove(home)?;
        if live && write.complete {
            self.restartable
                .insert(home.to_owned(), write.gateway_account_id);
        }
        Some(write.complete)
    }
}

#[cfg(any(unix, windows))]
static RESTART_NOTICES: LazyLock<Mutex<RestartNotices>> =
    LazyLock::new(|| Mutex::new(RestartNotices::default()));

#[cfg(target_os = "macos")]
pub(crate) struct SwitchRefresh {
    home: PathBuf,
}

#[cfg(target_os = "macos")]
impl SwitchRefresh {
    /// The account switch owns the profile lease until finish returns.
    pub(crate) fn begin(home: &Path) -> Option<Self> {
        if !crate::modules::config::get_user_config().codex_cli_auto_refresh_on_switch {
            return None;
        }
        let home = std::path::absolute(home).ok()?;
        let mut notices = RESTART_NOTICES
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        notices.switching.insert(home.clone(), None);
        Some(Self { home })
    }

    fn end(&self) -> Option<ProfileWrite> {
        RESTART_NOTICES
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .switching
            .remove(&self.home)
            .flatten()
    }

    pub(crate) async fn finish(self) {
        let write = RESTART_NOTICES
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .switching
            .get(&self.home)
            .and_then(Clone::clone);
        let Some(write) = write else {
            return;
        };
        if write.complete {
            match restart_if_unattached(&self.home).await {
                Ok(()) => {
                    self.end();
                    return;
                }
                Err(error) => crate::modules::logger::log_info(&format!(
                    "[Codex CLI daemon] Automatic refresh deferred: {error}"
                )),
            }
        }
        let write = self.end().unwrap_or(write);
        schedule_notice_with_context(&self.home, write.complete, write.gateway_account_id, true);
    }
}

#[cfg(target_os = "macos")]
impl Drop for SwitchRefresh {
    fn drop(&mut self) {
        if let Some(write) = self.end() {
            schedule_notice(&self.home, write.complete, write.gateway_account_id);
        }
    }
}

/// Schedule a bounded probe without delaying the switch. A partial write can
/// still change auth and show guidance, but only a complete write offers restart.
pub(crate) fn after_profile_write(codex_home: &Path, complete: bool) {
    schedule_notice(codex_home, complete, None);
}

pub(crate) fn after_provider_gateway_profile_write(codex_home: &Path, account_id: &str) {
    schedule_notice(codex_home, true, Some(account_id.to_owned()));
}

fn schedule_notice(codex_home: &Path, complete: bool, gateway_account_id: Option<String>) {
    schedule_notice_with_context(codex_home, complete, gateway_account_id, false);
}

fn schedule_notice_with_context(
    codex_home: &Path,
    complete: bool,
    gateway_account_id: Option<String>,
    auto_deferred: bool,
) {
    #[cfg(any(unix, windows))]
    {
        use tauri::Emitter;

        let Some(app) = crate::get_app_handle().cloned() else {
            return;
        };
        let Ok(home) = std::path::absolute(codex_home) else {
            return;
        };
        let write = ProfileWrite {
            complete,
            gateway_account_id,
        };
        let probe = {
            let mut notices = RESTART_NOTICES
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            #[cfg(target_os = "macos")]
            if let Some(pending) = notices.switching.get_mut(&home) {
                *pending = Some(write);
                notices.pending.remove(&home);
                notices.restartable.remove(&home);
                return;
            }
            let Some(probe) = notices.begin_probe(home.clone(), write) else {
                return;
            };
            probe
        };
        tauri::async_runtime::spawn(async move {
            let command = restart_notice(&home).await;
            let mut notices = RESTART_NOTICES
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            let Some(complete) = notices.finish_probe(&home, probe, command.is_some()) else {
                return;
            };
            // Publish in the same order as profile writes and restart permissions.
            if let Some(command) = command {
                let notice = serde_json::json!({
                    "command": command,
                    "codexHome": complete.then_some(&home),
                    "autoDeferred": auto_deferred,
                });
                if let Err(error) = app.emit("codex:cli-daemon-restart-required", notice) {
                    crate::modules::logger::log_warn(&format!(
                        "[Codex CLI daemon] Could not show restart notice: {error}"
                    ));
                }
            }
        });
    }
    #[cfg(not(any(unix, windows)))]
    let _ = (codex_home, complete, gateway_account_id, auto_deferred);
}

#[cfg(target_os = "macos")]
async fn restart_if_unattached(home: &Path) -> Result<(), String> {
    if !crate::modules::config::get_user_config().codex_cli_auto_refresh_on_switch {
        return Err("Automatic CLI refresh was disabled".into());
    }
    let pid = daemon_peer_pid(home)
        .await
        .ok_or("No verified CLI daemon")?;
    if !crate::commands::codex_instance::cli_resume::cli_allows_auto_switch(home.to_owned()).await {
        return Err("CLI account activity is present or unknown".into());
    }
    if !activity::remote_control_is_disabled(home, pid).await {
        return Err("CLI remote-control state is enabled or unknown".into());
    }
    let home_for_probe = home.to_owned();
    let (command, identity) = tokio::task::spawn_blocking(move || {
        let command = crate::modules::codex_wakeup::build_resolved_cli_command()?;
        let identity = crate::modules::codex_account::read_official_oauth_identity(&home_for_probe);
        let endpoint = home_for_probe
            .join(CONTROL_SOCKET)
            .canonicalize()
            .map_err(|error| error.to_string())?;
        let mut lsof = std::process::Command::new("/usr/sbin/lsof");
        lsof.args(["-nP", "-a", "-p", &pid.to_string(), "-U", "-F", "pftn"]);
        let result =
            crate::modules::process_timeout::output_with_timeout(&mut lsof, Duration::from_secs(2))
                .map_err(|error| error.to_string())?;
        if !result.status.success()
            || !result.stderr.is_empty()
            || !std::str::from_utf8(&result.stdout)
                .is_ok_and(|listing| daemon_has_no_client_sockets(listing, pid, &endpoint))
        {
            return Err("CLI clients are connected or could not be checked".into());
        }
        Ok::<_, String>((command, identity))
    })
    .await
    .map_err(|error| error.to_string())??;
    let next_pid = restart_daemon_from_pid(command.into(), home, pid).await?;
    if let Some(identity) = identity {
        if !matches!(
            activity::verify_workspace(home, next_pid, &identity).await?,
            RestartOutcome::WorkspaceConfirmed
        ) {
            return Err("CLI workspace could not be confirmed".into());
        }
    }
    Ok(())
}

#[cfg(target_os = "macos")]
fn daemon_has_no_client_sockets(listing: &str, pid: u32, endpoint: &Path) -> bool {
    let Some(endpoint) = endpoint.to_str() else {
        return false;
    };
    let mut fields = listing.lines();
    if fields.next() != Some(format!("p{pid}").as_str()) {
        return false;
    }
    let mut unix = false;
    let mut matches = 0;
    for line in fields {
        match line.as_bytes().first() {
            Some(b'f') => unix = false,
            Some(b't') if line == "tunix" => unix = true,
            Some(b'n') if unix && &line[1..] == endpoint => matches += 1,
            _ => {}
        }
    }
    matches == 1
}

#[cfg(any(unix, windows))]
#[tauri::command]
pub(crate) async fn codex_restart_cli_daemon(
    codex_home: PathBuf,
) -> Result<RestartOutcome, String> {
    use crate::modules::{codex_account, codex_local_access, codex_wakeup};

    let profile = codex_home.clone();
    let (_lease, command, gateway_account_id, identity) = tokio::task::spawn_blocking(move || {
        if !RESTART_NOTICES
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .restartable
            .contains_key(&profile)
        {
            return Err("No restart notice for this Codex profile".to_string());
        }
        let lease =
            codex_account::try_acquire_profile_mutation_lease(&profile, "cli-daemon-restart")?;
        let gateway_account_id = RESTART_NOTICES
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .restartable
            .get(&profile)
            .cloned()
            .ok_or("No restart notice for this Codex profile")?;
        if let Some(account_id) = gateway_account_id.as_deref() {
            if !codex_local_access::profile_uses_provider_gateway_account(&profile, account_id) {
                return Err("The Codex profile's provider changed after this notice".to_string());
            }
        }
        let command = codex_wakeup::build_resolved_cli_command()?;
        let identity = codex_account::read_official_oauth_identity(&profile);
        Ok::<_, String>((lease, command, gateway_account_id, identity))
    })
    .await
    .map_err(|error| error.to_string())??;
    if let Some(account_id) = gateway_account_id {
        if !daemon_is_listening(&codex_home).await {
            return Err("The Codex CLI daemon is no longer listening".into());
        }
        codex_local_access::ensure_provider_gateway_for_dir(&codex_home, &account_id).await?;
    }
    let pid = restart_daemon(command.into(), &codex_home).await?;
    let Some(identity) = identity else {
        return Ok(RestartOutcome::Restarted);
    };
    #[cfg(unix)]
    return activity::verify_workspace(&codex_home, pid, &identity).await;
    #[cfg(windows)]
    {
        let _ = (identity, pid);
        Ok(RestartOutcome::WorkspaceUnconfirmed)
    }
}

#[cfg(any(unix, windows))]
async fn restart_daemon(
    command: tokio::process::Command,
    codex_home: &Path,
) -> Result<u32, String> {
    let previous_pid = daemon_peer_pid(codex_home)
        .await
        .ok_or("The Codex CLI daemon owner could not be confirmed")?;
    restart_daemon_from_pid(command, codex_home, previous_pid).await
}

#[cfg(any(unix, windows))]
async fn restart_daemon_from_pid(
    command: tokio::process::Command,
    codex_home: &Path,
    previous_pid: u32,
) -> Result<u32, String> {
    let pid = run_daemon_restart_command(command, codex_home).await?;
    if pid == previous_pid {
        return Err(RESTART_UNCONFIRMED.into());
    }
    #[cfg(unix)]
    let ready = activity::managed_daemon_is_ready(codex_home, pid).await;
    #[cfg(windows)]
    let ready = daemon_peer_pid(codex_home).await == Some(pid);
    if !ready {
        return Err(RESTART_UNCONFIRMED.into());
    }
    Ok(pid)
}

#[cfg(any(unix, windows))]
async fn run_daemon_restart_command(
    mut command: tokio::process::Command,
    codex_home: &Path,
) -> Result<u32, String> {
    use std::process::Stdio;

    // Codex owns the graceful drain and startup deadlines. Killing this
    // command before the drain finishes can leave the old daemon stopped
    // without starting the replacement.
    let output = command
        .args(["app-server", "daemon", "restart"])
        .env("CODEX_HOME", codex_home)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .output()
        .await
        .map_err(|error| error.to_string())?;
    if !output.status.success() {
        return Err(format!("Codex CLI exited with {}", output.status));
    }

    #[derive(serde::Deserialize)]
    struct ResultRow {
        status: String,
        pid: u32,
    }
    let result: ResultRow =
        serde_json::from_slice(&output.stdout).map_err(|_| RESTART_UNCONFIRMED.to_string())?;
    if result.status != "restarted" || result.pid == 0 {
        return Err(RESTART_UNCONFIRMED.into());
    }
    Ok(result.pid)
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::net::UnixListener;
    use std::path::PathBuf;

    struct TestHome(PathBuf);

    impl TestHome {
        fn new() -> Self {
            // macOS limits Unix socket paths to 104 bytes.
            let path =
                PathBuf::from("/tmp").join(format!("cdx-notice-{}", uuid::Uuid::new_v4().simple()));
            std::fs::create_dir_all(path.join("app-server-control")).unwrap();
            Self(path)
        }

        fn socket(&self) -> PathBuf {
            self.0.join("app-server-control/app-server-control.sock")
        }
    }

    impl Drop for TestHome {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn automatic_refresh_requires_exactly_one_socket_for_the_control_path() {
        let endpoint = Path::new("/private/tmp/codex-daemon/a socket");
        let listener = "f33\ntunix\nn/private/tmp/codex-daemon/a socket\n";
        let other_fd = "f7\ntunix\nn->0x123\n";
        assert!(daemon_has_no_client_sockets(
            &format!("p111\n{other_fd}{listener}"),
            111,
            endpoint
        ));
        assert!(!daemon_has_no_client_sockets(
            &format!("p111\n{other_fd}{listener}{listener}"),
            111,
            endpoint
        ));
        assert!(!daemon_has_no_client_sockets(
            &format!("p111\n{other_fd}"),
            111,
            endpoint
        ));
        assert!(!daemon_has_no_client_sockets(
            &format!("p222\n{listener}"),
            111,
            endpoint
        ));
    }

    #[test]
    fn latest_write_controls_restart_even_when_the_previous_probe_is_pending() {
        let home = PathBuf::from("profile");
        let other = PathBuf::from("other-profile");
        let complete = |account: &str| ProfileWrite {
            complete: true,
            gateway_account_id: Some(account.to_owned()),
        };
        let incomplete = || ProfileWrite {
            complete: false,
            gateway_account_id: None,
        };
        let mut notices = RestartNotices::default();
        let other_probe = notices
            .begin_probe(other.clone(), complete("separate"))
            .unwrap();
        assert_eq!(notices.finish_probe(&other, other_probe, true), Some(true));

        let prior = notices
            .begin_probe(home.clone(), complete("previous"))
            .unwrap();
        assert!(notices.begin_probe(home.clone(), incomplete()).is_none());
        assert_eq!(notices.finish_probe(&home, prior, true), Some(false));
        assert!(!notices.restartable.contains_key(&home));

        let latest = notices.begin_probe(home.clone(), incomplete()).unwrap();
        assert!(notices
            .begin_probe(home.clone(), complete("latest"))
            .is_none());
        assert_eq!(notices.finish_probe(&home, prior, true), None);
        assert_eq!(notices.finish_probe(&home, latest, true), Some(true));
        assert_eq!(notices.restartable[&home].as_deref(), Some("latest"));

        let final_probe = notices.begin_probe(home.clone(), incomplete()).unwrap();
        assert!(!notices.restartable.contains_key(&home));
        assert_eq!(notices.finish_probe(&home, final_probe, false), Some(false));
        assert_eq!(notices.restartable[&other].as_deref(), Some("separate"));
    }

    #[tokio::test]
    async fn only_the_committed_profile_with_a_live_socket_gets_a_notice() {
        let home = TestHome::new();
        let other = TestHome::new();
        assert_eq!(restart_notice(&home.0).await, None);
        let listener = UnixListener::bind(home.socket()).unwrap();
        assert_eq!(
            restart_notice(&home.0).await,
            Some(format!(
                "CODEX_HOME='{}' codex app-server daemon restart",
                home.0.display()
            ))
        );
        assert_eq!(restart_notice(&other.0).await, None);
        drop(listener);
    }

    #[tokio::test]
    async fn an_exited_daemon_leaves_a_stale_socket_without_a_notice() {
        const CHILD_HOME: &str = "COCKPIT_TEST_EXITED_DAEMON_HOME";
        if let Some(home) = std::env::var_os(CHILD_HOME) {
            let listener = UnixListener::bind(
                Path::new(&home).join("app-server-control/app-server-control.sock"),
            )
            .unwrap();
            drop(listener);
            return;
        }
        let home = TestHome::new();
        // A concurrent Command::spawn can briefly inherit a listener even with
        // CLOEXEC. Create the stale socket in a child and wait for its exit, so
        // this process and its other children never own that listening FD.
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "modules::codex_cli_daemon::tests::an_exited_daemon_leaves_a_stale_socket_without_a_notice",
                "--nocapture",
            ])
            .env(CHILD_HOME, &home.0)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(home.socket().exists());
        assert_eq!(restart_notice(&home.0).await, None);
    }

    #[tokio::test]
    async fn long_home_notifies_only_for_the_daemon_behind_its_socket_link() {
        let home = TestHome::new();
        let other = TestHome::new();
        let long_home = home.0.join("long-profile".repeat(15));
        let control = long_home.join("app-server-control");
        std::fs::create_dir_all(&control).unwrap();
        let socket_link = control.join("app-server-control.sock");
        std::os::unix::fs::symlink(home.socket(), &socket_link).unwrap();
        let _other_listener = UnixListener::bind(other.socket()).unwrap();
        assert_eq!(restart_notice(&long_home).await, None);

        let _listener = UnixListener::bind(home.socket()).unwrap();
        assert_eq!(
            restart_notice(&long_home).await,
            Some(format!(
                "CODEX_HOME='{}' codex app-server daemon restart",
                long_home.display()
            )),
        );
    }

    #[tokio::test]
    async fn a_regular_file_or_overlong_socket_is_not_a_live_daemon() {
        let home = TestHome::new();
        std::fs::write(home.socket(), "not a socket").unwrap();
        assert_eq!(restart_notice(&home.0).await, None);
        assert_eq!(restart_notice(&home.0.join("x".repeat(150))).await, None);
    }

    #[tokio::test]
    async fn notice_quotes_the_exact_home_without_executing_it() {
        let mut home = TestHome::new();
        let special_home = home.0.with_extension("' $;`");
        std::fs::rename(&home.0, &special_home).unwrap();
        home.0 = special_home;
        let _listener = UnixListener::bind(home.socket()).unwrap();
        let command = restart_notice(&home.0).await.unwrap();
        // A local shell function captures the arguments; no Codex process runs.
        let output = std::process::Command::new("/bin/sh")
            .args([
                "-c",
                &format!("codex() {{ printf '%s\\n' \"$CODEX_HOME\" \"$@\"; }}; {command}"),
            ])
            .current_dir("/")
            .output()
            .unwrap();
        assert!(output.status.success());
        assert_eq!(
            String::from_utf8(output.stdout).unwrap(),
            format!("{}\napp-server\ndaemon\nrestart\n", home.0.display())
        );
    }

    #[tokio::test]
    async fn restart_runs_only_for_the_listening_profile_and_reports_process_failure() {
        let home = TestHome::new();
        let other = TestHome::new();
        let _listener = UnixListener::bind(home.socket()).unwrap();
        let output = home.0.join("invocation");
        let command = || {
            let mut command = tokio::process::Command::new("/bin/sh");
            command
                .args([
                    "-c",
                    r#"pid=$1; shift
printf '%s\n' "$CODEX_HOME" "$@" > "$0"
printf '{"status":"restarted","pid":%s}\n' "$pid""#,
                ])
                .arg(&output)
                .arg(std::process::id().to_string());
            command
        };
        assert!(restart_daemon(command(), &other.0).await.is_err());
        assert!(!output.exists());

        assert_eq!(
            restart_daemon(command(), &home.0).await.unwrap_err(),
            RESTART_UNCONFIRMED
        );
        assert_eq!(
            std::fs::read_to_string(&output).unwrap(),
            format!("{}\napp-server\ndaemon\nrestart\n", home.0.display())
        );

        let mut failing = tokio::process::Command::new("/bin/sh");
        failing.args(["-c", "exit 9"]);
        assert!(restart_daemon(failing, &home.0)
            .await
            .unwrap_err()
            .contains("9"));

        let mut silent = tokio::process::Command::new("/bin/sh");
        silent.args(["-c", "exit 0"]);
        assert_eq!(
            restart_daemon(silent, &home.0).await.unwrap_err(),
            RESTART_UNCONFIRMED
        );
    }

    #[tokio::test]
    async fn restart_reports_success_after_the_cli_finishes_draining() {
        let home = TestHome::new();
        let _listener = UnixListener::bind(home.socket()).unwrap();
        let mut command = tokio::process::Command::new("/bin/sh");
        command.args([
            "-c",
            r#"touch "$CODEX_HOME/draining"
               while [ ! -f "$CODEX_HOME/finish" ]; do sleep 0.01; done
               touch "$CODEX_HOME/restarted"
               echo '{"status":"restarted","pid":1234}'"#,
        ]);
        let restart_home = home.0.clone();
        let task =
            tokio::spawn(async move { run_daemon_restart_command(command, &restart_home).await });
        tokio::time::timeout(Duration::from_secs(2), async {
            while !home.0.join("draining").exists() {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("the CLI began the restart");
        assert!(!task.is_finished());
        std::fs::write(home.0.join("finish"), "").unwrap();
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(2), task)
                .await
                .unwrap()
                .unwrap()
                .unwrap(),
            1234
        );
        assert!(home.0.join("restarted").exists());
    }
}
