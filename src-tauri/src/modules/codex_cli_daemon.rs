//! Warn about a running CLI daemon after committing credentials. Never restart it:
//! the shared daemon can still be serving an active CLI session.

use std::path::Path;

#[cfg(any(target_os = "macos", all(test, unix)))]
async fn restart_notice(codex_home: &Path) -> Option<String> {
    let socket = codex_home.join("app-server-control/app-server-control.sock");
    let live = tokio::time::timeout(
        std::time::Duration::from_millis(250),
        tokio::net::UnixStream::connect(socket),
    )
    .await;
    if !matches!(live, Ok(Ok(_))) {
        return None;
    }
    // The terminal may have a different working directory. Shell quoting is only
    // for the displayed command; Cockpit never executes this command.
    let home = std::path::absolute(codex_home).ok()?;
    let home = home.to_str()?.replace('\'', "'\"'\"'");
    Some(format!(
        "CODEX_HOME='{home}' codex app-server daemon restart"
    ))
}

/// Schedule a bounded, profile-scoped probe after auth.json/keyring was written.
/// The caller must invoke this immediately after the auth commit, even when later
/// configuration writes can still fail. It never delays the account switch.
pub(crate) fn after_auth_commit(codex_home: &Path) {
    #[cfg(target_os = "macos")]
    {
        use std::collections::HashSet;
        use std::path::PathBuf;
        use std::sync::{LazyLock, Mutex};
        use tauri::Emitter;

        static IN_FLIGHT: LazyLock<Mutex<HashSet<PathBuf>>> =
            LazyLock::new(|| Mutex::new(HashSet::new()));
        let Some(app) = crate::get_app_handle().cloned() else {
            return;
        };
        let Ok(home) = std::path::absolute(codex_home) else {
            return;
        };
        {
            let mut pending = IN_FLIGHT.lock().unwrap_or_else(|error| error.into_inner());
            if !pending.insert(home.clone()) {
                return;
            }
        }
        tauri::async_runtime::spawn(async move {
            if let Some(command) = restart_notice(&home).await {
                if let Err(error) = app.emit("codex:cli-daemon-restart-required", command) {
                    crate::modules::logger::log_warn(&format!(
                        "[Codex CLI daemon] Could not show restart notice: {error}"
                    ));
                }
            }
            IN_FLIGHT
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .remove(&home);
        });
    }
    #[cfg(not(target_os = "macos"))]
    let _ = codex_home;
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
        assert!(home.socket().exists());
        assert_eq!(restart_notice(&home.0).await, None);
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
}
