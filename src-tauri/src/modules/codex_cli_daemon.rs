//! Restart the shared CLI daemon after a manual account switch on macOS.

use std::path::Path;

/// The daemon control socket is scoped to CODEX_HOME (Codex CLI 0.157.1).
/// A successful connection distinguishes a live daemon from a stale socket file.
#[cfg(any(target_os = "macos", all(test, unix)))]
async fn is_running(codex_home: &Path) -> bool {
    let socket = codex_home.join("app-server-control/app-server-control.sock");
    matches!(
        tokio::time::timeout(
            std::time::Duration::from_millis(250),
            tokio::net::UnixStream::connect(socket),
        )
        .await,
        Ok(Ok(_))
    )
}

#[cfg(any(target_os = "macos", test))]
fn restart_command(codex_home: &Path) -> std::io::Result<String> {
    // The user's terminal may have a different working directory than Cockpit.
    let codex_home = std::path::absolute(codex_home)?;
    // Quote the exact profile, including spaces, apostrophes and shell metacharacters.
    let home = codex_home.to_string_lossy().replace('\'', "'\"'\"'");
    Ok(format!(
        "CODEX_HOME='{home}' codex app-server daemon restart"
    ))
}

#[cfg(any(target_os = "macos", all(test, unix)))]
fn build_restart_process(binary: &Path, node: Option<&Path>) -> std::process::Command {
    let mut command = if let Some(node) = node {
        let mut command = std::process::Command::new(node);
        command.arg(binary);
        command
    } else {
        std::process::Command::new(binary)
    };
    command.args(["app-server", "daemon", "restart"]);
    command
}

#[cfg(target_os = "macos")]
async fn resolve_restart_process() -> Result<std::process::Command, String> {
    // Reuse configured CLI/Node paths and discovery for Finder's minimal PATH.
    // A timed-out blocking probe can finish later, but must never restart a daemon.
    let runtime = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        tokio::task::spawn_blocking(crate::modules::codex_wakeup::resolve_cli_runtime),
    )
    .await
    .map_err(|_| "CLI discovery timed out".to_string())?
    .map_err(|_| "CLI discovery task failed".to_string())??;
    let mut command = build_restart_process(
        Path::new(&runtime.binary_path),
        runtime.node_path.as_deref().map(Path::new),
    );
    crate::modules::process::apply_managed_proxy_env_to_command(&mut command);
    Ok(command)
}

#[cfg(any(target_os = "macos", all(test, unix)))]
async fn wait_for_restart(
    child: &mut tokio::process::Child,
    timeout: std::time::Duration,
) -> Result<std::process::ExitStatus, String> {
    match tokio::time::timeout(timeout, child.wait()).await {
        Ok(result) => result.map_err(|error| format!("Could not wait for CLI: {error}")),
        Err(_) => {
            let _ = child.kill().await;
            let _ = child.wait().await;
            Err("Daemon restart timed out".to_string())
        }
    }
}

#[cfg(any(target_os = "macos", all(test, unix)))]
async fn restart_with<F, Fut>(
    codex_home: &Path,
    resolve_process: F,
    timeout: std::time::Duration,
) -> Option<String>
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = Result<std::process::Command, String>>,
{
    // Do not start an unused daemon or restart a different profile's daemon.
    if !is_running(codex_home).await {
        return None;
    }
    let result: Result<(), String> = async {
        let home = std::path::absolute(codex_home)
            .map_err(|_| "Could not resolve CODEX_HOME".to_string())?;
        let mut command = tokio::process::Command::from(resolve_process().await?);
        // No shell: keep profile names with spaces/metacharacters intact. Discard
        // child output so credentials cannot leak into logs or inherited pipes.
        command
            .env("CODEX_HOME", &home)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .kill_on_drop(true);
        let spawn_guard =
            crate::modules::app_lifecycle::acquire_process_spawn_guard("Codex CLI daemon")
                .map_err(|error| error.to_string())?;
        let child = command.spawn();
        drop(spawn_guard);
        let mut child = child.map_err(|error| format!("Could not start CLI: {error}"))?;
        let status = wait_for_restart(&mut child, timeout).await?;
        if !status.success() {
            return Err(format!("Daemon restart exited with {status}"));
        }
        if !is_running(&home).await {
            return Err("Daemon control socket is unavailable after restart".to_string());
        }
        Ok(())
    }
    .await;
    match result {
        Ok(()) => {
            crate::modules::logger::log_info("[Codex CLI daemon] Restarted after account switch");
            None
        }
        Err(error) => {
            crate::modules::logger::log_warn(&format!(
                "[Codex CLI daemon] Automatic restart failed: {error}"
            ));
            // Keep the fallback even if the failed restart stopped the old daemon.
            restart_command(codex_home).ok()
        }
    }
}

/// Restart only after credentials have been committed, including partial switches.
/// This can interrupt connected CLI sessions. A failed restart keeps the account
/// switch result intact and returns the existing manual command as a fallback.
pub async fn restart_after_auth_commit(codex_home: &Path) -> Option<String> {
    #[cfg(target_os = "macos")]
    {
        restart_with(
            codex_home,
            resolve_restart_process,
            std::time::Duration::from_secs(15),
        )
        .await
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = codex_home;
        None
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    use std::os::unix::net::UnixListener;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    struct TestHome(PathBuf);

    impl TestHome {
        fn new() -> Self {
            static NEXT_ID: AtomicUsize = AtomicUsize::new(0);
            // Keep the Unix socket path below macOS's sockaddr_un limit.
            let path = PathBuf::from("/tmp").join(format!(
                "cockpit-daemon-{}-{}",
                std::process::id(),
                NEXT_ID.fetch_add(1, Ordering::Relaxed),
            ));
            std::fs::create_dir_all(path.join("app-server-control")).unwrap();
            Self(path)
        }

        fn socket(&self) -> PathBuf {
            self.0.join("app-server-control/app-server-control.sock")
        }

        fn fake_cli(&self, script: &str) -> PathBuf {
            let path = self.0.join("fake codex");
            std::fs::write(&path, format!("#!/bin/sh\n{script}\n")).unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
            path
        }
    }

    impl Drop for TestHome {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[tokio::test]
    async fn detects_only_a_live_daemon_in_the_switched_home() {
        let home = TestHome::new();
        let other_home = TestHome::new();
        assert!(!is_running(&home.0).await);

        let listener = UnixListener::bind(home.socket()).unwrap();
        assert!(is_running(&home.0).await);
        assert!(!is_running(&other_home.0).await);

        drop(listener);
        assert!(home.socket().exists());
        assert!(!is_running(&home.0).await);
        assert_eq!(restart_after_auth_commit(&home.0).await, None);
    }

    #[tokio::test]
    async fn a_regular_file_is_not_a_running_daemon() {
        let home = TestHome::new();
        std::fs::write(home.socket(), b"not a socket").unwrap();
        assert!(!is_running(&home.0).await);
    }

    #[tokio::test]
    async fn absent_stale_or_other_profile_daemon_never_starts_cli() {
        let home = TestHome::new();
        let other_home = TestHome::new();
        let _other_listener = UnixListener::bind(other_home.socket()).unwrap();
        for stale in [false, true] {
            if stale {
                drop(UnixListener::bind(home.socket()).unwrap());
            }
            assert_eq!(
                restart_with(
                    &home.0,
                    || async { panic!("must not resolve CLI without a live daemon in this home") },
                    Duration::from_secs(2),
                )
                .await,
                None
            );
        }
    }

    #[tokio::test]
    async fn successful_restart_uses_exact_home_and_arguments_without_a_notice() {
        let mut home = TestHome::new();
        // Keep the socket below macOS's path limit while exercising shell characters.
        let special_home = home.0.with_extension("a' $;`b`");
        std::fs::rename(&home.0, &special_home).unwrap();
        home.0 = special_home;
        let _listener = UnixListener::bind(home.socket()).unwrap();
        let cli = home.fake_cli("printf '%s\\n' \"$CODEX_HOME\" \"$@\" > \"$CODEX_HOME/received\"");
        assert_eq!(
            restart_with(
                &home.0,
                || async { Ok(build_restart_process(&cli, None)) },
                Duration::from_secs(2),
            )
            .await,
            None
        );
        assert_eq!(
            std::fs::read_to_string(home.0.join("received")).unwrap(),
            format!("{}\napp-server\ndaemon\nrestart\n", home.0.display())
        );
    }

    #[tokio::test]
    async fn node_wrapper_passes_script_before_daemon_arguments() {
        let home = TestHome::new();
        let _listener = UnixListener::bind(home.socket()).unwrap();
        let node = home.fake_cli("printf '%s\\n' \"$@\" > \"$CODEX_HOME/received\"");
        let script = home.0.join("node modules/codex.js");
        assert_eq!(
            restart_with(
                &home.0,
                || async { Ok(build_restart_process(&script, Some(&node))) },
                Duration::from_secs(2),
            )
            .await,
            None
        );
        assert_eq!(
            std::fs::read_to_string(home.0.join("received")).unwrap(),
            format!("{}\napp-server\ndaemon\nrestart\n", script.display())
        );
    }

    #[tokio::test]
    async fn missing_cli_or_failed_spawn_returns_manual_notice() {
        let home = TestHome::new();
        let _listener = UnixListener::bind(home.socket()).unwrap();
        let expected = Some(restart_command(&home.0).unwrap());
        assert_eq!(
            restart_with(
                &home.0,
                || async { Err("CLI not installed".to_string()) },
                Duration::from_secs(2),
            )
            .await,
            expected
        );
        assert_eq!(
            restart_with(
                &home.0,
                || async { Ok(build_restart_process(&home.0.join("missing"), None)) },
                Duration::from_secs(2),
            )
            .await,
            expected
        );
    }

    #[tokio::test]
    async fn failed_restart_keeps_notice_even_after_old_socket_is_removed() {
        let home = TestHome::new();
        let _listener = UnixListener::bind(home.socket()).unwrap();
        let cli =
            home.fake_cli("rm \"$CODEX_HOME/app-server-control/app-server-control.sock\"\nexit 7");
        assert_eq!(
            restart_with(
                &home.0,
                || async { Ok(build_restart_process(&cli, None)) },
                Duration::from_secs(2),
            )
            .await,
            Some(restart_command(&home.0).unwrap())
        );
        assert!(!home.socket().exists());
    }

    #[tokio::test]
    async fn zero_exit_without_live_daemon_still_requests_manual_restart() {
        let home = TestHome::new();
        let _listener = UnixListener::bind(home.socket()).unwrap();
        let cli = home.fake_cli("rm \"$CODEX_HOME/app-server-control/app-server-control.sock\"");
        assert_eq!(
            restart_with(
                &home.0,
                || async { Ok(build_restart_process(&cli, None)) },
                Duration::from_secs(2),
            )
            .await,
            Some(restart_command(&home.0).unwrap())
        );
    }

    #[tokio::test]
    async fn timed_out_restart_returns_manual_notice() {
        let home = TestHome::new();
        let _listener = UnixListener::bind(home.socket()).unwrap();
        assert_eq!(
            restart_with(
                &home.0,
                || async {
                    let mut command = std::process::Command::new("/bin/sleep");
                    command.arg("30");
                    Ok(command)
                },
                Duration::from_millis(50),
            )
            .await,
            Some(restart_command(&home.0).unwrap())
        );
    }

    #[tokio::test]
    async fn timed_out_child_is_killed_and_reaped() {
        let mut child = tokio::process::Command::new("/bin/sleep")
            .arg("30")
            .kill_on_drop(true)
            .spawn()
            .unwrap();
        let started = std::time::Instant::now();
        assert_eq!(
            wait_for_restart(&mut child, Duration::from_millis(50))
                .await
                .unwrap_err(),
            "Daemon restart timed out"
        );
        assert!(started.elapsed() < Duration::from_secs(5));
        let status = child
            .try_wait()
            .unwrap()
            .expect("child must already be reaped");
        assert!(!status.success());
        assert!(child.id().is_none());
    }
}

#[cfg(test)]
mod command_tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn restart_command_quotes_the_switched_profile() {
        assert_eq!(
            restart_command(Path::new("/tmp/Alice's Codex/$profile")).unwrap(),
            "CODEX_HOME='/tmp/Alice'\"'\"'s Codex/$profile' codex app-server daemon restart",
        );
    }

    #[test]
    fn restart_command_resolves_relative_home_against_cockpit_working_directory() {
        let relative = Path::new("profiles/team-a");
        let absolute = std::env::current_dir().unwrap().join(relative);
        assert_eq!(
            restart_command(relative).unwrap(),
            restart_command(&absolute).unwrap(),
        );
    }

    #[cfg(unix)]
    #[test]
    fn restart_command_keeps_profile_and_arguments_in_a_different_terminal_directory() {
        let relative = Path::new("Alice's Codex/$profile;$(echo wrong)`echo wrong`");
        let expected_home = std::env::current_dir().unwrap().join(relative);
        // A shell function captures arguments only; no real Codex is started.
        let script = format!(
            "codex() {{ printf '%s\\n' \"$CODEX_HOME\" \"$@\"; }}; {}",
            restart_command(relative).unwrap(),
        );
        let output = std::process::Command::new("/bin/sh")
            .arg("-c")
            .arg(script)
            .current_dir("/")
            .output()
            .unwrap();
        assert!(output.status.success());
        assert_eq!(
            String::from_utf8(output.stdout).unwrap(),
            format!("{}\napp-server\ndaemon\nrestart\n", expected_home.display()),
        );
    }

    #[cfg(not(target_os = "macos"))]
    #[tokio::test]
    async fn unsupported_platform_does_not_request_a_daemon_restart() {
        assert_eq!(
            restart_after_auth_commit(Path::new("profiles/team-a")).await,
            None
        );
    }
}
