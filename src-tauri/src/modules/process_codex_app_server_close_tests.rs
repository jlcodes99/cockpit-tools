use super::close_captured_codex_direct_app_servers;
use std::os::unix::process::{CommandExt, ExitStatusExt};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

struct FixtureProcess(Child);

impl FixtureProcess {
    fn start(ignore_term: bool) -> Self {
        let mut command = Command::new("/bin/sleep");
        command
            .arg("60")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        unsafe {
            command.pre_exec(move || {
                let handler = if ignore_term {
                    libc::SIG_IGN
                } else {
                    libc::SIG_DFL
                };
                if libc::signal(libc::SIGTERM, handler) == libc::SIG_ERR {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
        Self(command.spawn().expect("start test-only background process"))
    }
}

impl Drop for FixtureProcess {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[test]
fn normal_shutdown_does_not_require_force_or_manual_retry() {
    let mut target = FixtureProcess::start(false);
    close_captured_codex_direct_app_servers(&[target.0.id()], 1).unwrap();
    assert_eq!(target.0.wait().unwrap().signal(), Some(libc::SIGTERM));
}

#[test]
fn term_timeout_automatically_force_stops_only_captured_processes() {
    let mut target = FixtureProcess::start(true);
    let mut control = FixtureProcess::start(true);
    let started = Instant::now();
    let pid = target.0.id();
    close_captured_codex_direct_app_servers(&[pid, pid], 1).unwrap();
    assert!(started.elapsed() < Duration::from_secs(5));
    assert_eq!(target.0.wait().unwrap().signal(), Some(libc::SIGKILL));
    assert!(control.0.try_wait().unwrap().is_none());
}

#[test]
fn empty_and_already_exited_targets_are_successful() {
    close_captured_codex_direct_app_servers(&[], 1).unwrap();
    let mut target = FixtureProcess::start(false);
    target.0.kill().unwrap();
    target.0.wait().unwrap();
    close_captured_codex_direct_app_servers(&[target.0.id()], 1).unwrap();
}
