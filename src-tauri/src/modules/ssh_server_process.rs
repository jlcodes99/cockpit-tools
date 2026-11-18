//! Own the SSH child and its stdin for the full command lifetime.
use std::process::{Output, Stdio};
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStderr, Command};
use tokio::time::timeout;

pub(crate) async fn output(
    mut command: Command,
    duration: Duration,
    payload: Option<String>,
) -> Result<Output, String> {
    command
        .kill_on_drop(true)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if payload.is_some() {
        command.stdin(Stdio::piped());
    }
    // Include writing stdin: a stalled SSH handshake can leave write_all
    // pending before wait_with_output ever begins. Dropping the timed-out
    // future also terminates its owned child and releases both output pipes.
    timeout(duration, async {
        let mut child = command
            .spawn()
            .map_err(|e| format!("ssh_binary_missing: {e}"))?;
        if let Some(payload) = payload {
            let mut stdin = child
                .stdin
                .take()
                .ok_or("ssh_connection_failed: stdin unavailable")?;
            stdin
                .write_all(payload.as_bytes())
                .await
                .map_err(|e| format!("ssh_connection_failed: {e}"))?;
            drop(stdin);
        }
        child
            .wait_with_output()
            .await
            .map_err(|e| format!("ssh_connection_failed: {e}"))
    })
    .await
    .map_err(|_| "ssh_connection_failed: SSH command timed out".to_string())?
}

pub(super) struct ReverseForward {
    child: Child,
    remote_port: u16,
}

impl ReverseForward {
    pub(super) fn remote_port(&self) -> u16 {
        self.remote_port
    }

    pub(super) fn is_running(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(None))
    }
}

pub(super) async fn reverse_forward(
    mut command: Command,
    local_port: u16,
    duration: Duration,
) -> Result<ReverseForward, String> {
    let mut child = command
        .kill_on_drop(true)
        // The remote session waits for EOF. Even a crashed parent releases its pipe.
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| format!("ssh_binary_missing: {error}"))?;
    let mut stderr = BufReader::new(child.stderr.take().ok_or("SSH stderr unavailable")?);
    let remote_port = timeout(duration, wait_for_forward(&mut stderr, local_port))
        .await
        .map_err(|_| "SSH reverse forwarding timed out".to_string())??;
    // Continue draining until the tunnel is closed so diagnostics cannot block ssh.
    tokio::spawn(async move {
        let _ = tokio::io::copy(&mut stderr, &mut tokio::io::sink()).await;
    });
    Ok(ReverseForward { child, remote_port })
}

async fn wait_for_forward(
    stderr: &mut BufReader<ChildStderr>,
    local_port: u16,
) -> Result<u16, String> {
    // OpenSSH reports the server-selected port after sshd acknowledges the request.
    let destination = format!(" for remote forward to 127.0.0.1:{local_port}");
    let mut buffer = Vec::new();
    let mut diagnostic = "SSH closed before confirming remote forwarding".to_string();
    loop {
        buffer.clear();
        if stderr
            .read_until(b'\n', &mut buffer)
            .await
            .map_err(|e| e.to_string())?
            == 0
        {
            return Err(diagnostic);
        }
        let line = String::from_utf8_lossy(&buffer);
        let line = line.trim();
        // GatewayPorts notices describe sshd's binding policy. A confirmed port
        // remains usable through the remote loopback address under that policy.
        if let Some(port) = allocated_forward_port(line, &destination) {
            return Ok(port);
        }
        if !line.is_empty() && !line.starts_with("debug") {
            diagnostic = line.to_string();
        }
    }
}

fn allocated_forward_port(line: &str, destination: &str) -> Option<u16> {
    let port = line
        .strip_prefix("Allocated port ")?
        .strip_suffix(destination)?;
    port.parse::<std::num::NonZeroU16>()
        .ok()
        .map(|port| port.get())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;

    fn fixture(mode: &str) -> Command {
        let mut command = Command::new(std::env::current_exe().unwrap());
        let test = format!("{}::child_process_fixture", module_path!());
        command.args([
            "--exact",
            test.split_once("::").unwrap().1,
            "--ignored",
            "--nocapture",
        ]);
        command.env("COCKPIT_SSH_PROCESS_FIXTURE", mode);
        #[cfg(windows)]
        command.creation_flags(0x0800_0000);
        command
    }

    #[test]
    #[ignore = "child process fixture; invoked by process ownership tests"]
    fn child_process_fixture() {
        match std::env::var("COCKPIT_SSH_PROCESS_FIXTURE").as_deref() {
            Ok("echo") => {
                let mut payload = String::new();
                std::io::stdin().read_to_string(&mut payload).unwrap();
                println!("SSH_PROCESS_FIXTURE:{payload}");
            }
            Ok("stall") => std::thread::sleep(Duration::from_secs(4)),
            Ok("forward") => {
                eprintln!("Allocated port 45001 for remote forward to 127.0.0.1:9001");
                eprintln!("Allocated port 45002 for remote forward to 127.0.0.1:9000");
                std::thread::sleep(Duration::from_secs(4));
            }
            Ok("forward-denied") => {
                eprintln!("Error: remote port forwarding failed for listen port 0");
                eprintln!("debug1: connection is closing");
                std::process::exit(255);
            }
            Ok("forward-exposed") => {
                eprintln!("debug1: Remote: Forwarding listen address \"127.0.0.1\" overridden by server GatewayPorts");
                eprintln!("Allocated port 45002 for remote forward to 127.0.0.1:9000");
                std::thread::sleep(Duration::from_secs(4));
            }
            Ok("forward-policy-only") => {
                eprintln!("debug1: Remote: Forwarding listen address \"127.0.0.1\" overridden by server GatewayPorts");
                std::process::exit(255);
            }
            Ok("forward-drop") => {
                eprintln!("Allocated port 45002 for remote forward to 127.0.0.1:9000");
                std::thread::sleep(Duration::from_millis(800));
                let marker = std::env::var_os("COCKPIT_SSH_PROCESS_MARKER").unwrap();
                std::fs::write(marker, "SSH child survived").unwrap();
            }
            _ => {}
        }
    }

    #[tokio::test]
    async fn reverse_forward_waits_for_the_port_of_the_requested_service() {
        let mut forward = reverse_forward(fixture("forward"), 9000, Duration::from_secs(3))
            .await
            .expect("receive the service's allocated port");
        assert_eq!(forward.remote_port(), 45002);
        assert!(forward.is_running());
    }

    #[tokio::test]
    async fn reverse_forward_reports_refusal_and_times_out_without_an_acknowledgement() {
        let denied = reverse_forward(fixture("forward-denied"), 9000, Duration::from_secs(3))
            .await
            .err()
            .expect("forwarding must be confirmed");
        assert!(denied.contains("remote port forwarding failed"), "{denied}");
        let stalled = reverse_forward(fixture("stall"), 9000, Duration::from_millis(200))
            .await
            .err()
            .expect("unconfirmed forwarding must time out");
        assert!(stalled.contains("timed out"), "{stalled}");
    }

    #[tokio::test]
    async fn reverse_forward_accepts_a_confirmed_port_with_the_servers_bind_policy() {
        let mut forward = reverse_forward(fixture("forward-exposed"), 9000, Duration::from_secs(3))
            .await
            .expect("the server's address policy still allows forwarding");
        assert_eq!(forward.remote_port(), 45002);
        assert!(forward.is_running());
    }

    #[tokio::test]
    async fn reverse_forward_requires_confirmation_after_a_server_policy_notice() {
        let error = reverse_forward(fixture("forward-policy-only"), 9000, Duration::from_secs(3))
            .await
            .err()
            .expect("a policy notice does not confirm an allocated port");
        assert!(error.contains("closed before confirming"), "{error}");
    }

    #[tokio::test]
    async fn dropping_reverse_forward_terminates_its_child() {
        let marker = std::env::temp_dir().join(format!(
            "cockpit-ssh-child-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
        ));
        let mut command = fixture("forward-drop");
        command.env("COCKPIT_SSH_PROCESS_MARKER", &marker);
        let forward = reverse_forward(command, 9000, Duration::from_secs(3))
            .await
            .expect("ready forward");
        drop(forward);
        tokio::time::sleep(Duration::from_secs(1)).await;
        let child_survived = marker.exists();
        if child_survived {
            let _ = std::fs::remove_file(&marker);
        }
        assert!(!child_survived);
    }

    #[tokio::test]
    async fn sends_payload_closes_stdin_and_collects_output() {
        let result = output(
            fixture("echo"),
            Duration::from_secs(3),
            Some("fixture-only".into()),
        )
        .await
        .unwrap();
        assert!(result.status.success());
        assert!(
            String::from_utf8_lossy(&result.stdout).contains("SSH_PROCESS_FIXTURE:fixture-only")
        );
    }

    #[test]
    fn stalled_stdin_obeys_the_command_deadline() {
        let started = std::time::Instant::now();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let error = runtime
            .block_on(output(
                fixture("stall"),
                Duration::from_millis(200),
                Some("x".repeat(32 * 1024 * 1024)),
            ))
            .unwrap_err();
        drop(runtime);
        assert!(error.contains("timed out"), "{error}");
        assert!(started.elapsed() < Duration::from_secs(2));
    }
}
