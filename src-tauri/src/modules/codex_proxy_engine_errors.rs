//! Reduce bounded engine stderr to fixed codes; raw lines are never logged or persisted.
use std::sync::{
    atomic::{AtomicU8, Ordering},
    Arc,
};
use tokio::io::AsyncReadExt;
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
pub fn read(
    mut stderr: impl tokio::io::AsyncRead + Unpin + Send + 'static,
    status: Arc<AtomicU8>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut buffer = [0u8; 512];
        let mut line = Vec::with_capacity(4096);
        let mut overflow = false;
        loop {
            let Ok(n) = stderr.read(&mut buffer).await else {
                break;
            };
            if n == 0 {
                break;
            }
            for byte in &buffer[..n] {
                if *byte == b'\n' {
                    if !overflow {
                        let c = classify(&line);
                        if c != 0 {
                            status.store(c, Ordering::Relaxed);
                        }
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
#[cfg(test)]
mod tests {
    use super::*;
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
