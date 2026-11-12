//! Own the SSH child and its stdin for the full command lifetime.
use std::process::{Output, Stdio};
use std::time::Duration;
use tokio::io::AsyncWriteExt;
use tokio::process::Command;
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
            _ => {}
        }
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
