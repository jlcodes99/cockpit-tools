//! Reduce bounded engine stderr to fixed codes; raw lines are never logged or persisted.
use regex::Regex;
use std::sync::{
    atomic::{AtomicU8, Ordering},
    Arc, LazyLock, Mutex, Weak,
};
use tokio::{io::AsyncReadExt, process::Child};
pub fn classify(line: &[u8]) -> u8 {
    let s = String::from_utf8_lossy(line).to_ascii_lowercase();
    if s.contains("no ech config") {
        1
    } else if s.contains("certificate") || s.contains("x509:") {
        2
    } else if s.contains("no such interface")
        || s.contains("bind interface")
        || s.contains("device not found")
    {
        3
    } else if s.contains("dns")
        && (s.contains("failed") || s.contains("timeout") || s.contains("no such host"))
    {
        4
    } else if s.contains("handshake") {
        5
    } else if s.contains("connection refused") {
        6
    } else {
        0
    }
}
pub fn code(value: u8) -> Option<&'static str> {
    match value {
        1 => Some("PROXY_ECH_DNS"),
        2 => Some("PROXY_TLS_FAILED"),
        3 => Some("PROXY_INTERFACE_FAILED"),
        4 => Some("PROXY_DNS_FAILED"),
        5 => Some("PROXY_HANDSHAKE_FAILED"),
        6 => Some("PROXY_CONNECTION_REFUSED"),
        _ => None,
    }
}
fn transport_code(line: &[u8]) -> Option<&'static str> {
    let s = String::from_utf8_lossy(line).to_ascii_lowercase();
    if s.contains("connection reset") {
        Some("connection_reset")
    } else if s.contains("broken pipe") {
        Some("broken_pipe")
    } else if s.contains("unexpected eof") {
        Some("unexpected_eof")
    } else if s.contains("timeout") || s.contains("timed out") {
        Some("timeout")
    } else {
        code(classify(line)).or_else(|| {
            if s.contains("level=warning") || s.contains("level=error") || s.contains("level=fatal")
            {
                Some("unclassified_engine_warning")
            } else {
                None
            }
        })
    }
}

fn local_source_port(line: &[u8]) -> Option<u16> {
    // Only the loopback source preceding Mihomo's arrow; never a destination or URL.
    static SOURCE: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r#"(?:^|[\s"])(?:127\.0\.0\.1|\[::1\]):([0-9]{1,5})\s+-->"#)
            .expect("static proxy source regex")
    });
    let text = String::from_utf8_lossy(line);
    SOURCE.captures(&text)?.get(1)?.as_str().parse().ok()
}

fn record(line: &[u8], status: &AtomicU8, tunnel_id: uuid::Uuid, source: &'static str) {
    let c = classify(line);
    if c != 0 {
        status.store(c, Ordering::Relaxed);
    }
    if let Some(code) = transport_code(line) {
        super::logger::log_warn(&format!(
            "[ProxyEngine] transport_error: tunnel_id={tunnel_id}, source={source}, code={code}, upstream_source_port={:?}", local_source_port(line)
        ));
    }
}

pub fn read(
    mut stderr: impl tokio::io::AsyncRead + Unpin + Send + 'static,
    status: Arc<AtomicU8>,
    tunnel_id: uuid::Uuid,
    source: &'static str,
    process: Option<Weak<Mutex<Child>>>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut buffer = [0u8; 512];
        let mut line = Vec::with_capacity(4096);
        let mut overflow = false;
        loop {
            let n = match stderr.read(&mut buffer).await {
                Ok(n) => n,
                Err(error) => {
                    super::logger::log_warn(&format!("[ProxyEngine] log_reader_end: tunnel_id={tunnel_id}, source={source}, outcome=io_error, io_kind={:?}, os_code={:?}", error.kind(), error.raw_os_error()));
                    break;
                }
            };
            if n == 0 {
                if overflow {
                    super::logger::log_warn(&format!("[ProxyEngine] log_line_discarded: tunnel_id={tunnel_id}, source={source}, reason=over_4096_bytes"));
                }
                if !overflow && !line.is_empty() {
                    record(&line, &status, tunnel_id, source);
                }
                super::logger::log_info(&format!("[ProxyEngine] log_reader_end: tunnel_id={tunnel_id}, source={source}, outcome=eof"));
                observe_process_exit(process.as_ref(), tunnel_id, source).await;
                break;
            }
            for byte in &buffer[..n] {
                if *byte == b'\n' {
                    if !overflow {
                        record(&line, &status, tunnel_id, source);
                    }
                    if overflow {
                        super::logger::log_warn(&format!("[ProxyEngine] log_line_discarded: tunnel_id={tunnel_id}, source={source}, reason=over_4096_bytes"));
                    }
                    line.clear();
                    overflow = false;
                } else if line.len() < 4096 {
                    line.push(*byte)
                } else {
                    overflow = true;
                }
            }
        }
    })
}
// EOF on a pipe may precede process reaping briefly. No standing poller or strong lease.
async fn observe_process_exit(
    process: Option<&Weak<Mutex<Child>>>,
    id: uuid::Uuid,
    source: &'static str,
) {
    let Some(process) = process else {
        return;
    };
    for _ in 0..10 {
        let state = process.upgrade().and_then(|child| {
            let mut child = child.try_lock().ok()?;
            child.try_wait().ok().flatten()
        });
        if let Some(status) = state {
            super::logger::log_warn(&format!(
                "[ProxyEngine] process_exit: tunnel_id={id}, source={source}, exit_status={status}"
            ));
            return;
        }
        if process.strong_count() == 0 {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
    super::logger::log_info(&format!(
        "[ProxyEngine] process_exit_unconfirmed: tunnel_id={id}, source={source}"
    ));
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(unix)]
    #[tokio::test]
    async fn closed_output_reports_child_exit_without_holding_a_lease() {
        #[derive(Clone, Default)]
        struct Capture(Arc<Mutex<Vec<u8>>>);
        impl std::io::Write for Capture {
            fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
                self.0.lock().unwrap().extend_from_slice(bytes);
                Ok(bytes.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        let capture = Capture::default();
        let writer = capture.clone();
        let subscriber = tracing_subscriber::fmt()
            .without_time()
            .with_ansi(false)
            .with_writer(move || writer.clone())
            .finish();
        let _guard = tracing::subscriber::set_default(subscriber);
        let mut child = tokio::process::Command::new("/bin/sh")
            .args([
                "-c",
                "printf 'level=warning msg=secret-unknown-error\\n'; exit 7",
            ])
            .stdout(std::process::Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .unwrap();
        let stdout = child.stdout.take().unwrap();
        let child = Arc::new(Mutex::new(child));
        let reader = read(
            stdout,
            Arc::new(AtomicU8::new(0)),
            uuid::Uuid::nil(),
            "test",
            Some(Arc::downgrade(&child)),
        );
        tokio::time::timeout(std::time::Duration::from_secs(2), reader)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(Arc::strong_count(&child), 1);
        assert_eq!(
            child.lock().unwrap().try_wait().unwrap().unwrap().code(),
            Some(7)
        );
        let logs = String::from_utf8(capture.0.lock().unwrap().clone()).unwrap();
        assert!(logs.contains("process_exit:") && logs.contains("outcome=eof"));
        assert!(logs.contains("unclassified_engine_warning"));
        assert!(!logs.contains("secret-unknown-error"));
    }

    #[tokio::test]
    async fn reader_handles_final_line_and_oversized_input() {
        let status = Arc::new(AtomicU8::new(0));
        read(
            &b"handshake failed secret"[..],
            status.clone(),
            uuid::Uuid::nil(),
            "test",
            None,
        )
        .await
        .unwrap();
        assert_eq!(status.load(Ordering::Relaxed), 5);
        let input = format!("{}connection refused\n", "x".repeat(4096));
        read(
            std::io::Cursor::new(input),
            status.clone(),
            uuid::Uuid::nil(),
            "test",
            None,
        )
        .await
        .unwrap();
        assert_eq!(status.load(Ordering::Relaxed), 5);
    }

    #[test]
    fn transport_errors_are_fixed_and_do_not_change_status_codes() {
        for (raw, expected) in [
            (
                "read tcp private-host: connection reset by peer token=secret",
                "connection_reset",
            ),
            ("write: broken pipe secret", "broken_pipe"),
            ("unexpected EOF secret", "unexpected_eof"),
            ("i/o timeout secret", "timeout"),
        ] {
            assert_eq!(transport_code(raw.as_bytes()), Some(expected));
            assert_eq!(classify(raw.as_bytes()), 0);
        }
        assert_eq!(transport_code(b"unknown token=secret"), None);
    }

    #[test]
    fn source_correlation_extracts_only_loopback_source_ports() {
        assert_eq!(
            local_source_port(b"msg=\"[TCP] dial 127.0.0.1:45678 --> private.example:443 error\""),
            Some(45678)
        );
        assert_eq!(
            local_source_port(b"[TCP] [::1]:45679 --> private.example:443"),
            Some(45679)
        );
        for raw in [
            "10.0.0.1:1234 --> host",
            "127.0.0.1:1234/path",
            "x127.0.0.1:1234 --> host",
            "host --> 127.0.0.1:1234",
            "127.0.0.1:99999 --> host",
        ] {
            assert_eq!(local_source_port(raw.as_bytes()), None);
        }
        assert_eq!(
            transport_code(b"level=warning msg=secret-unknown-error"),
            Some("unclassified_engine_warning")
        );
    }

    #[test]
    fn only_fixed_codes_leave_reader() {
        for (raw, expected) in [
            ("no ECH config found in DNS records token=secret", 1),
            ("x509: certificate expired secret", 2),
            ("lookup DNS failed", 4),
            ("handshake failed private-host", 5),
            ("unknown secret", 0),
        ] {
            assert_eq!(classify(raw.as_bytes()), expected);
            assert!(!code(expected).unwrap_or("").contains("secret"));
        }
    }
}
