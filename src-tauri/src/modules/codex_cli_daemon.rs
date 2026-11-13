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
async fn daemon_is_listening(codex_home: &Path) -> bool {
    let socket = codex_home.join(CONTROL_SOCKET);
    let live = tokio::time::timeout(Duration::from_millis(250), async {
        // Newer Codex releases point long profile paths at a short runtime
        // socket. Resolve the link before constructing the Unix socket address.
        let socket = if std::os::unix::net::SocketAddr::from_pathname(&socket).is_err() {
            tokio::fs::canonicalize(socket).await?
        } else {
            socket
        };
        tokio::net::UnixStream::connect(socket).await
    })
    .await;
    matches!(live, Ok(Ok(_)))
}

#[cfg(windows)]
async fn daemon_is_listening(codex_home: &Path) -> bool {
    use socket2::{Domain, SockAddr, Socket, Type};

    let Ok(Ok(control_dir)) = tokio::time::timeout(
        Duration::from_millis(250),
        tokio::fs::canonicalize(codex_home.join("app-server-control")),
    )
    .await
    else {
        return false;
    };
    tokio::task::spawn_blocking(move || {
        let socket = control_dir.join("app-server-control.sock");
        let Some(path) = socket.to_str() else {
            return false;
        };
        let path = crate::modules::process::normalize_windows_user_facing_path(path);
        let Ok(address) = SockAddr::unix(path) else {
            return false;
        };
        Socket::new(Domain::UNIX, Type::STREAM, None)
            .and_then(|socket| socket.connect_timeout(&address, Duration::from_millis(250)))
            .is_ok()
    })
    .await
    .unwrap_or(false)
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
static RESTARTABLE_PROFILES: LazyLock<Mutex<HashMap<PathBuf, Option<String>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// Schedule a bounded probe without delaying the switch. A partial write can
/// still change auth and show guidance, but only a complete write offers restart.
pub(crate) fn after_profile_write(codex_home: &Path, complete: bool) {
    schedule_notice(codex_home, complete, None);
}

pub(crate) fn after_provider_gateway_profile_write(codex_home: &Path, account_id: &str) {
    schedule_notice(codex_home, true, Some(account_id.to_owned()));
}

fn schedule_notice(codex_home: &Path, complete: bool, gateway_account_id: Option<String>) {
    #[cfg(any(unix, windows))]
    {
        use tauri::Emitter;

        type PendingProbes = HashMap<(PathBuf, bool), Option<String>>;
        static IN_FLIGHT: LazyLock<Mutex<PendingProbes>> =
            LazyLock::new(|| Mutex::new(HashMap::new()));
        let Some(app) = crate::get_app_handle().cloned() else {
            return;
        };
        let Ok(home) = std::path::absolute(codex_home) else {
            return;
        };
        {
            let mut pending = IN_FLIGHT.lock().unwrap_or_else(|error| error.into_inner());
            if pending
                .insert((home.clone(), complete), gateway_account_id)
                .is_some()
            {
                return;
            }
        }
        tauri::async_runtime::spawn(async move {
            let command = restart_notice(&home).await;
            {
                let mut pending = IN_FLIGHT.lock().unwrap_or_else(|error| error.into_inner());
                let gateway_account_id = pending.remove(&(home.clone(), complete)).flatten();
                if complete && command.is_some() {
                    RESTARTABLE_PROFILES
                        .lock()
                        .unwrap_or_else(|error| error.into_inner())
                        .insert(home.clone(), gateway_account_id);
                }
            }
            if let Some(command) = command {
                let notice = serde_json::json!({
                    "command": command,
                    "codexHome": complete.then_some(&home),
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
    let _ = (codex_home, complete, gateway_account_id);
}

#[cfg(any(unix, windows))]
#[tauri::command]
pub(crate) async fn codex_restart_cli_daemon(codex_home: PathBuf) -> Result<(), String> {
    use crate::modules::{codex_account, codex_local_access, codex_wakeup};

    let profile = codex_home.clone();
    let (_lease, command, gateway_account_id) = tokio::task::spawn_blocking(move || {
        if !RESTARTABLE_PROFILES
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .contains_key(&profile)
        {
            return Err("No restart notice for this Codex profile".to_string());
        }
        let lease =
            codex_account::try_acquire_profile_mutation_lease(&profile, "cli-daemon-restart")?;
        let gateway_account_id = RESTARTABLE_PROFILES
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .get(&profile)
            .cloned()
            .ok_or("No restart notice for this Codex profile")?;
        if let Some(account_id) = gateway_account_id.as_deref() {
            if !codex_local_access::profile_uses_provider_gateway_account(&profile, account_id) {
                return Err("The Codex profile's provider changed after this notice".to_string());
            }
        }
        let command = codex_wakeup::build_resolved_cli_command()?;
        Ok::<_, String>((lease, command, gateway_account_id))
    })
    .await
    .map_err(|error| error.to_string())??;
    if let Some(account_id) = gateway_account_id {
        if !daemon_is_listening(&codex_home).await {
            return Err("The Codex CLI daemon is no longer listening".into());
        }
        codex_local_access::ensure_provider_gateway_for_dir(&codex_home, &account_id).await?;
    }
    restart_daemon(command.into(), &codex_home, Duration::from_secs(45)).await
}

#[cfg(any(unix, windows))]
async fn restart_daemon(
    mut command: tokio::process::Command,
    codex_home: &Path,
    timeout: Duration,
) -> Result<(), String> {
    use std::process::Stdio;

    if !daemon_is_listening(codex_home).await {
        return Err("The Codex CLI daemon is no longer listening".into());
    }
    let status = tokio::time::timeout(
        timeout,
        command
            .args(["app-server", "daemon", "restart"])
            .env("CODEX_HOME", codex_home)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .status(),
    )
    .await
    .map_err(|_| "The Codex CLI restart command timed out".to_string())?
    .map_err(|error| error.to_string())?;
    if !status.success() {
        return Err(format!("Codex CLI exited with {status}"));
    }
    Ok(())
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
                .args(["-c", "printf '%s\\n' \"$CODEX_HOME\" \"$@\" > \"$0\""])
                .arg(&output);
            command
        };
        assert!(restart_daemon(command(), &other.0, Duration::from_secs(2))
            .await
            .is_err());
        assert!(!output.exists());

        restart_daemon(command(), &home.0, Duration::from_secs(2))
            .await
            .unwrap();
        assert_eq!(
            std::fs::read_to_string(&output).unwrap(),
            format!("{}\napp-server\ndaemon\nrestart\n", home.0.display())
        );

        let mut failing = tokio::process::Command::new("/bin/sh");
        failing.args(["-c", "exit 9"]);
        assert!(restart_daemon(failing, &home.0, Duration::from_secs(2))
            .await
            .unwrap_err()
            .contains("9"));
        let mut hanging = tokio::process::Command::new("/bin/sh");
        hanging.args(["-c", "exec /bin/sleep 30"]);
        assert!(restart_daemon(hanging, &home.0, Duration::from_millis(50))
            .await
            .unwrap_err()
            .contains("timed out"));
    }
}
