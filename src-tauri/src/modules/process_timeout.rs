//! Bounded subprocess helpers for registry/keychain probes that must not hang UI.

use std::io::{self, Read};
use std::process::{Command, Output, Stdio};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

/// Run `command` and wait up to `timeout`. On timeout, kill the child and return
/// `io::ErrorKind::TimedOut`.
///
/// The wall-clock bound covers both process lifetime and stdout/stderr drain.
/// If a descendant holds an inherited pipe after the parent exits, join is aborted
/// when `timeout` elapses (reader threads are detached).
pub fn output_with_timeout(command: &mut Command, timeout: Duration) -> io::Result<Output> {
    let program = command.get_program().to_string_lossy().into_owned();
    let spawn_guard = crate::modules::app_lifecycle::acquire_process_spawn_guard(&program)?;
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let child = command.spawn();
    drop(spawn_guard);
    let mut child = child?;
    let stdout_reader = child.stdout.take().map(|mut pipe| {
        std::thread::spawn(move || {
            let mut output = Vec::new();
            pipe.read_to_end(&mut output).map(|_| output)
        })
    });
    let stderr_reader = child.stderr.take().map(|mut pipe| {
        std::thread::spawn(move || {
            let mut output = Vec::new();
            pipe.read_to_end(&mut output).map(|_| output)
        })
    });
    let deadline = Instant::now() + timeout;

    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => {
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    // Do not join readers on the kill path: a surviving descendant may
                    // still hold an inherited pipe, and the timeout path must stay bounded.
                    drop(stdout_reader);
                    drop(stderr_reader);
                    return Err(io::Error::new(
                        io::ErrorKind::TimedOut,
                        format!("process timed out after {:?}", timeout),
                    ));
                }
                std::thread::sleep(Duration::from_millis(20));
            }
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                drop(stdout_reader);
                drop(stderr_reader);
                return Err(error);
            }
        }
    };

    let stdout = join_reader_with_deadline(stdout_reader, deadline)?;
    let stderr = join_reader_with_deadline(stderr_reader, deadline)?;

    Ok(Output {
        status,
        stdout,
        stderr,
    })
}

fn join_reader_with_deadline(
    reader: Option<JoinHandle<io::Result<Vec<u8>>>>,
    deadline: Instant,
) -> io::Result<Vec<u8>> {
    let Some(handle) = reader else {
        return Ok(Vec::new());
    };
    loop {
        if handle.is_finished() {
            return handle
                .join()
                .map_err(|_| {
                    io::Error::new(io::ErrorKind::Other, "process output reader panicked")
                })?
                .map_err(|error| error);
        }
        if Instant::now() >= deadline {
            // Detach the reader thread; it will exit when the pipe is eventually closed.
            drop(handle);
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "process output drain timed out",
            ));
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn completes_fast_command() {
        let mut command = if cfg!(windows) {
            let mut c = Command::new("cmd");
            c.args(["/C", "echo ok"]);
            c
        } else {
            let mut c = Command::new("echo");
            c.arg("ok");
            c
        };
        let output = output_with_timeout(&mut command, Duration::from_secs(2)).expect("output");
        assert!(output.status.success());
        assert!(!output.stdout.is_empty() || cfg!(windows));
    }

    #[test]
    fn times_out_long_command() {
        let mut command = if cfg!(windows) {
            let mut c = Command::new("cmd");
            c.args(["/C", "ping -n 5 127.0.0.1 >nul"]);
            c
        } else {
            let mut c = Command::new("sleep");
            c.arg("5");
            c
        };
        let err = output_with_timeout(&mut command, Duration::from_millis(200)).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::TimedOut);
    }

    #[test]
    fn drains_large_output_without_false_timeout() {
        let mut command = if cfg!(windows) {
            let mut c = Command::new("cmd");
            c.args(["/C", "for /L %i in (1,1,20000) do @echo 1234567890"]);
            c
        } else {
            let mut c = Command::new("sh");
            c.args(["-c", "yes 1234567890 | head -c 200000"]);
            c
        };
        let output = output_with_timeout(&mut command, Duration::from_secs(5))
            .expect("large output command");
        assert!(output.status.success());
        assert!(output.stdout.len() >= 100_000);
    }
}
